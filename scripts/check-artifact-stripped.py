#!/usr/bin/env python3
"""Fail the build if a published artifact still carries debug information.

`[profile.release]` used to set `strip = "symbols"`, so every binary left
the compiler already stripped and nothing had to check. RFC 0377 took that
out, because `sentry-cli debug-files upload` needs the symbols the strip was
removing, and moved the strip into `release.yml` — after the upload, before
the artifact is staged.

That trade is the reason this script exists. A profile setting is one line in
one file and applies to every build; a pipeline step applies to whichever
command happens to run before it. Reorder two steps, add a build job, change
a bundler flag, and the published binary quietly carries full DWARF again —
the build is green, the signature is fine, and the only symptom is a download
several times its old size.

So the verdict comes from the finished file, the way
`crates/outl-mobile/scripts/android-apk-page-alignment.py` reads the built
APK rather than trusting the two build steps that decide its alignment.

What counts as a failure:

  * ELF   — any `.debug*` / `.zdebug*` section, or a `.symtab`
            (binutils `strip` removes both; a stripped ELF keeps `.dynsym`)
  * Mach-O — a `__DWARF` segment, a `__debug_*` section, a debug map left
            in the symbol table, or local symbols beyond the one `strip`
            itself adds. Neither count goes to zero: Apple's `strip` writes
            a `radr://5614542` N_OSO entry *meaning* "this was stripped", so
            a stripped binary measures one STAB and one local (checked on
            `/bin/ls` and on a `strip -x`-ed build) while an unstripped one
            carries an N_OSO per object file.
  * PE    — a `.debug*` section, or a COFF symbol table. MSVC writes the
            debug info to a separate `.pdb`, so a Windows build passes
            without a strip step; the check is what confirms that, instead
            of the comment that used to.

`--allow-symbols` keeps the DWARF and debug-map rules and drops the
symbol-table ones, for an artifact whose binary cannot be stripped after the
fact.

**No workflow step passes it today**, and that is worth stating rather than
leaving as a puzzle. It exists because two of the callers check an artifact
whose strip happens inside a tool we do not drive — `xcodebuild`'s
`STRIP_INSTALLED_PRODUCT` for the IPA, AGP's strip task for the APK — so if
either turns out to leave local symbols behind, the fix is this flag on that
one step rather than new code written against a red release.

Containers are opened, not trusted: `.tar.gz`, `.zip`, `.apk`, `.ipa`, and a
directory are walked for anything whose magic says it is a binary. A path that
is *meant* to hold debug info (`.dSYM`, `.dwp`, `.pdb`, `.debug`) is skipped —
those are what we just uploaded.

Finding no binary at all is a failure too. A check that passes because it
looked at nothing is worse than no check: it reports the thing it did not do.

Usage: check-artifact-stripped.py [--allow-symbols] <artifact|dir>...
"""

import os
import struct
import sys
import tarfile
import zipfile

# Reading a member of an APK or a dmg's app binary means holding it in memory;
# a few hundred MB would be the whole artifact, which is never a binary.
MAX_BLOB = 512 * 1024 * 1024

TAR_SUFFIXES = (".tar.gz", ".tgz", ".tar")
ZIP_SUFFIXES = (".zip", ".apk", ".ipa", ".aab", ".jar")

# Files whose entire job is to carry debug info.
DEBUG_COMPANIONS = (".dsym", ".dwp", ".pdb", ".debug", ".dwarf", ".bcsymbolmap")


def _u(fmt, buf, off):
    return struct.unpack_from(fmt, buf, off)


# ---------------------------------------------------------------- ELF


def check_elf(name, blob, allow_symbols):
    bits = blob[4]
    endian = "<" if blob[5] == 1 else ">"
    if bits == 2:
        (shoff,) = _u(endian + "Q", blob, 0x28)
        shentsize, shnum, shstrndx = _u(endian + "HHH", blob, 0x3A)
        name_off, type_off, off_off, size_off = 0, 4, 0x18, 0x20
        word = endian + "Q"
    else:
        (shoff,) = _u(endian + "I", blob, 0x20)
        shentsize, shnum, shstrndx = _u(endian + "HHH", blob, 0x2E)
        name_off, type_off, off_off, size_off = 0, 4, 0x10, 0x14
        word = endian + "I"

    if not shoff or not shnum or shstrndx >= shnum:
        return [f"{name}: no section headers to read (truncated or corrupt ELF)"]

    def header(i):
        return shoff + i * shentsize

    strtab = header(shstrndx)
    (str_off,) = _u(word, blob, strtab + off_off)
    (str_size,) = _u(word, blob, strtab + size_off)
    names = blob[str_off : str_off + str_size]

    failures = []
    for i in range(shnum):
        h = header(i)
        (n,) = _u(endian + "I", blob, h + name_off)
        end = names.find(b"\0", n)
        section = names[n : end if end != -1 else None].decode("utf-8", "replace")
        if section.startswith((".debug", ".zdebug")):
            (size,) = _u(word, blob, h + size_off)
            failures.append(f"{name}: carries {section} ({size} bytes of DWARF)")
        elif section == ".symtab" and not allow_symbols:
            # SHT_SYMTAB == 2; `strip` drops the section outright.
            (kind,) = _u(endian + "I", blob, h + type_off)
            if kind == 2:
                failures.append(
                    f"{name}: carries a .symtab, so `strip` never ran on it"
                )
    return failures


# ------------------------------------------------------------- Mach-O

MH_MAGIC_64, MH_MAGIC_32 = 0xFEEDFACF, 0xFEEDFACE
FAT_MAGIC, FAT_MAGIC_64 = 0xCAFEBABE, 0xCAFEBABF
LC_SEGMENT, LC_SEGMENT_64, LC_SYMTAB, LC_DYSYMTAB = 0x01, 0x19, 0x02, 0x0B

# `n_type & N_STAB` marks a debug-map entry: one N_OSO per object file, each
# naming its path. That is both the debug info dsymutil consumes and a leak of
# the build machine's layout, so it must not reach a user.
N_STAB = 0xE0

# The entry Apple's `strip` leaves *because* it stripped. It is a STAB and it
# counts as a local symbol, so both tolerances below are one, not zero.
STRIP_MARKER = "radr://"


def _macho_magic(blob, off=0):
    if len(blob) < off + 4:
        return None
    for endian in ("<", ">"):
        (magic,) = _u(endian + "I", blob, off)
        if magic in (MH_MAGIC_64, MH_MAGIC_32):
            return endian, magic
    return None


def check_macho_slice(name, blob, base, allow_symbols):
    endian, magic = _macho_magic(blob, base)
    wide = magic == MH_MAGIC_64
    ncmds = _u(endian + "I", blob, base + 16)[0]
    off = base + (32 if wide else 28)

    failures = []
    for _ in range(ncmds):
        if off + 8 > len(blob):
            return [f"{name}: load commands run past the end of the file"]
        cmd, cmdsize = _u(endian + "II", blob, off)
        if cmdsize < 8:
            return [f"{name}: load command of size {cmdsize} (corrupt)"]

        if cmd in (LC_SEGMENT, LC_SEGMENT_64):
            seg = blob[off + 8 : off + 24].rstrip(b"\0").decode("utf-8", "replace")
            if seg == "__DWARF":
                failures.append(f"{name}: carries a __DWARF segment")
            else:
                nsects_off = off + (64 if cmd == LC_SEGMENT_64 else 48)
                (nsects,) = _u(endian + "I", blob, nsects_off)
                sect_size = 80 if cmd == LC_SEGMENT_64 else 68
                first = off + (72 if cmd == LC_SEGMENT_64 else 56)
                for s in range(nsects):
                    at = first + s * sect_size
                    if at + 16 > len(blob):
                        break
                    sect = blob[at : at + 16].rstrip(b"\0").decode("utf-8", "replace")
                    if sect.startswith("__debug"):
                        failures.append(f"{name}: carries {seg},{sect}")
        elif cmd == LC_SYMTAB:
            failures.extend(_macho_debug_map(name, blob, endian, off, wide, base))
        elif cmd == LC_DYSYMTAB and not allow_symbols:
            # `strip -x` keeps the externals and takes every local, so the
            # local count is what says whether it ran.
            _, nlocalsym = _u(endian + "II", blob, off + 8)
            if nlocalsym > 1:
                failures.append(
                    f"{name}: {nlocalsym} local symbols left, so `strip -x` never ran on it"
                )
        off += cmdsize
    return failures


def _macho_debug_map(name, blob, endian, off, wide, base):
    """Every STAB in the symbol table, minus the one `strip` adds itself.

    `base` is not optional. A slice of a universal binary is a whole Mach-O
    whose offsets count from its own start, so reading them against the file
    lands in another slice's bytes — which decoded as 72 debug-map entries
    with empty names on a `/bin/ls` Apple ships stripped.
    """
    symoff, nsyms, stroff, strsize = _u(endian + "IIII", blob, off + 8)
    symoff, stroff = base + symoff, base + stroff
    entry = 16 if wide else 12
    if symoff + nsyms * entry > len(blob):
        return [f"{name}: symbol table runs past the end of the file"]

    strings = blob[stroff : stroff + strsize]
    objects = []
    for i in range(nsyms):
        at = symoff + i * entry
        strx, n_type = _u(endian + "IB", blob, at)
        if not n_type & N_STAB:
            continue
        end = strings.find(b"\0", strx)
        sym = strings[strx : end if end != -1 else None].decode("utf-8", "replace")
        if not sym.startswith(STRIP_MARKER):
            objects.append(sym)

    if not objects:
        return []
    shown = ", ".join(objects[:3])
    return [
        (
            f"{name}: carries a debug map of {len(objects)} entries naming the "
            f"build machine's object files ({shown}…)"
        )
    ]


def check_macho(name, blob, allow_symbols):
    (magic,) = _u(">I", blob, 0)
    if magic not in (FAT_MAGIC, FAT_MAGIC_64):
        return check_macho_slice(name, blob, 0, allow_symbols)

    # A universal binary — what `--target universal-apple-darwin` produces.
    # `0xcafebabe` is also a Java class file's magic, so every slice is
    # confirmed to be Mach-O before any of them is judged.
    (narch,) = _u(">I", blob, 4)
    wide = magic == FAT_MAGIC_64
    entry = 32 if wide else 20
    if not 0 < narch <= 16 or 8 + narch * entry > len(blob):
        return [
            f"{name}: fat header claims {narch} slices, which does not read as Mach-O"
        ]

    offsets = []
    for i in range(narch):
        at = 8 + i * entry
        (off,) = _u(">Q" if wide else ">I", blob, at + 8)
        if not _macho_magic(blob, off):
            return [f"{name}: fat slice {i} at offset {off} is not Mach-O"]
        offsets.append(off)

    failures = []
    for i, off in enumerate(offsets):
        for line in check_macho_slice(name, blob, off, allow_symbols):
            failures.append(f"{line} (slice {i})")
    return failures


# ----------------------------------------------------------------- PE


def check_pe(name, blob, allow_symbols):
    (lfanew,) = _u("<I", blob, 0x3C)
    if lfanew + 24 > len(blob) or blob[lfanew : lfanew + 4] != b"PE\0\0":
        # A plain DOS stub, or something else starting with "MZ". Not ours.
        return []
    coff = lfanew + 4
    nsections = _u("<H", blob, coff + 2)[0]
    sym_ptr, nsyms = _u("<II", blob, coff + 8)
    opt_size = _u("<H", blob, coff + 16)[0]

    failures = []
    if sym_ptr and nsyms and not allow_symbols:
        failures.append(f"{name}: carries a COFF symbol table ({nsyms} symbols)")

    table = coff + 20 + opt_size
    for i in range(nsections):
        at = table + i * 40
        if at + 40 > len(blob):
            break
        section = blob[at : at + 8].rstrip(b"\0").decode("utf-8", "replace")
        if section.startswith(".debug"):
            (raw,) = _u("<I", blob, at + 16)
            failures.append(f"{name}: carries {section} ({raw} bytes of DWARF)")
    return failures


# ------------------------------------------------------------ dispatch


def inspect(name, blob, allow_symbols):
    """Judge one blob, or return None when it is not a binary at all."""
    if len(blob) < 64:
        return None
    if blob[:4] == b"\x7fELF":
        return "ELF", check_elf(name, blob, allow_symbols)
    if _macho_magic(blob) or _u(">I", blob, 0)[0] in (FAT_MAGIC, FAT_MAGIC_64):
        return "Mach-O", check_macho(name, blob, allow_symbols)
    if blob[:2] == b"MZ":
        return "PE", check_pe(name, blob, allow_symbols)
    return None


def is_debug_companion(path):
    lowered = path.lower()
    return any(part.endswith(DEBUG_COMPANIONS) for part in lowered.split("/"))


def walk(path, allow_symbols, results):
    """Feed every binary reachable from `path` to `inspect`."""
    lowered = path.lower()

    if os.path.isdir(path):
        for root, dirs, files in os.walk(path):
            dirs[:] = [d for d in dirs if not is_debug_companion(d)]
            for f in sorted(files):
                walk(os.path.join(root, f), allow_symbols, results)
        return

    if is_debug_companion(lowered):
        return

    if lowered.endswith(TAR_SUFFIXES):
        with tarfile.open(path) as tar:
            for member in tar.getmembers():
                if not member.isfile() or member.size > MAX_BLOB:
                    continue
                if is_debug_companion(member.name):
                    continue
                handle = tar.extractfile(member)
                if handle:
                    judge(
                        f"{path}:{member.name}", handle.read(), allow_symbols, results
                    )
        return

    if lowered.endswith(ZIP_SUFFIXES):
        with zipfile.ZipFile(path) as archive:
            for info in archive.infolist():
                if info.is_dir() or info.file_size > MAX_BLOB:
                    continue
                if is_debug_companion(info.filename):
                    continue
                with archive.open(info) as handle:
                    judge(
                        f"{path}:{info.filename}", handle.read(), allow_symbols, results
                    )
        return

    if os.path.getsize(path) > MAX_BLOB:
        return
    with open(path, "rb") as handle:
        judge(path, handle.read(), allow_symbols, results)


def judge(name, blob, allow_symbols, results):
    verdict = inspect(name, blob, allow_symbols)
    if verdict is None:
        return
    kind, failures = verdict
    results["checked"].append(f"{kind} {name} ({len(blob)} bytes)")
    results["failures"].extend(failures)


USAGE = "usage: {} [--allow-symbols] <artifact|dir>..."


def main():
    flags = sys.argv[1:]
    # `--help` before the path check: without it the help flag is read as an
    # artifact name and comes back as "no such artifact: --help", which is the
    # least useful thing to tell somebody debugging a red release job.
    if any(f in ("-h", "--help") for f in flags):
        print(USAGE.format(sys.argv[0]))
        print(__doc__ or "")
        return
    args = [a for a in flags if a != "--allow-symbols"]
    allow_symbols = "--allow-symbols" in flags
    if not args:
        sys.exit(USAGE.format(sys.argv[0]))

    results = {"checked": [], "failures": []}
    for path in args:
        if not os.path.exists(path):
            sys.exit(f"::error::no such artifact: {path}")
        walk(path, allow_symbols, results)

    for line in results["checked"]:
        print(f"checked {line}")

    if not results["checked"]:
        sys.exit(
            f"::error::found no binary inside {', '.join(args)} — this check "
            "reported nothing because it read nothing, which is not a pass"
        )

    if results["failures"]:
        for line in results["failures"]:
            print(f"::error::debug info in a published artifact: {line}")
        sys.exit(
            "this artifact ships debug information to users; the strip step in "
            "release.yml did not run, or ran before the bundle it was meant to "
            "cover (see RFC 0377 section 5)"
        )

    scope = "no DWARF" if allow_symbols else "no DWARF and no symbol table"
    print(f"every binary checked carries {scope}")


if __name__ == "__main__":
    main()
