#!/usr/bin/env python3
"""Tests for `scripts/check-artifact-stripped.py`.

The script is the only thing in the release pipeline that reads a finished
artifact back, so a false verdict either way is a release that goes red for
nothing or one that ships DWARF. Each case below is a shape a real release
artifact has: built by hand here so the suite runs anywhere with a stock
Python and no toolchain.

Run: python3 scripts/tests/test_check_artifact_stripped.py
"""

import importlib.util
import io
import os
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "check-artifact-stripped.py")

spec = importlib.util.spec_from_file_location("check_artifact_stripped", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

SHT_PROGBITS, SHT_SYMTAB, SHT_STRTAB = 1, 2, 3
SHF_ALLOC = 0x2
SHF_MERGE_STRINGS = 0x30


def elf64(sections):
    """A little-endian ELF64 with only section headers.

    `sections` is a list of `(name, sh_type, sh_flags, size)`. The section
    header string table is appended last, and its index goes in `e_shstrndx`.
    """
    names = b"\0"
    offsets = []
    for name, *_ in sections:
        offsets.append(len(names))
        names += name.encode() + b"\0"
    shstrtab_name = len(names)
    names += b".shstrtab\0"

    header_size = 64
    body = bytearray(names)
    data_off = header_size
    payload_off = data_off + len(body)
    payloads = []
    for _, _, _, size in sections:
        payloads.append(payload_off + len(body) - len(names))
        body += b"\0" * size

    shoff = header_size + len(body)
    shentsize = 64
    entries = [b"\0" * shentsize]  # SHN_UNDEF
    for (name, kind, flags, size), n_off, p_off in zip(sections, offsets, payloads):
        entries.append(
            struct.pack("<IIQQQQIIQQ", n_off, kind, flags, 0, p_off, size, 0, 0, 1, 0)
        )
    entries.append(
        struct.pack(
            "<IIQQQQIIQQ",
            shstrtab_name, SHT_STRTAB, 0, 0, data_off, len(names), 0, 0, 1, 0,
        )
    )
    shnum = len(entries)

    ident = b"\x7fELF" + bytes([2, 1, 1]) + b"\0" * 9
    header = ident + struct.pack(
        "<HHIQQQIHHHHHH",
        3, 0x3E, 1, 0, 0, shoff, 0, header_size, 0, 0, shentsize, shnum, shnum - 1,
    )
    return bytes(header) + bytes(body) + b"".join(entries)


def java_class(major=52):
    """A Java class file header. Its magic is the same 0xCAFEBABE as a fat
    Mach-O, and the minor/major version read as a fat slice count."""
    return struct.pack(">IHH", 0xCAFEBABE, 0, major) + b"\0" * 120


def verdict(path):
    results = {"checked": [], "failures": [], "env_failures": []}
    check.walk(path, False, results)
    return results


class BuildEnvTest(unittest.TestCase):
    STRIPPED = [(".text", SHT_PROGBITS, SHF_ALLOC | 0x4, 32)]

    def test_clean_binary_passes(self):
        blob = elf64(self.STRIPPED) + b"PATH\0HOME\0"
        self.assertEqual(check.check_build_env("outl", blob), [])

    def test_each_marker_fails(self):
        for marker in check.BUILD_ENV_MARKERS:
            with self.subTest(marker=marker):
                blob = elf64(self.STRIPPED) + marker + b"=123\0"
                failures = check.check_build_env("outl", blob)
                self.assertEqual(len(failures), 1)
                self.assertIn(marker.decode(), failures[0])

    def test_library_inside_apk_is_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            apk = os.path.join(tmp, "app.apk")
            with zipfile.ZipFile(apk, "w") as archive:
                archive.writestr(
                    "lib/arm64-v8a/liboutl.so",
                    elf64(self.STRIPPED) + b"RUNNER_TEMP=/tmp\0",
                )
            results = verdict(apk)
        self.assertEqual(results["failures"], [])
        self.assertEqual(len(results["env_failures"]), 1)
        self.assertIn("liboutl.so", results["env_failures"][0])


class GateExitTest(unittest.TestCase):
    """The verdict is only worth what the exit code does with it.

    Everything above calls `walk` or `check_build_env` and reads the
    `results` dict, so the whole of `main` — which categories are
    reported, which ones end the job, what the success line claims —
    was reachable by no test. Deleting the env branch left the suite
    green and the gate printing "no build environment" over an
    artifact that carried one. These run the script the way
    `release.yml` does, as a process, and read its exit code.
    """

    STRIPPED = [(".text", SHT_PROGBITS, SHF_ALLOC | 0x4, 32)]
    UNSTRIPPED = STRIPPED + [
        (".debug_line", SHT_PROGBITS, 0, 64),
        (".symtab", SHT_SYMTAB, 0, 48),
    ]

    def run_gate(self, blob, *flags):
        with tempfile.TemporaryDirectory() as tmp:
            binary = os.path.join(tmp, "outl")
            with open(binary, "wb") as handle:
                handle.write(blob)
            return subprocess.run(
                [sys.executable, SCRIPT, *flags, binary],
                capture_output=True,
                text=True,
            )

    def test_clean_binary_exits_zero(self):
        done = self.run_gate(elf64(self.STRIPPED) + b"PATH=/usr/bin\0")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("no build environment", done.stdout)

    def test_build_env_marker_exits_nonzero(self):
        done = self.run_gate(elf64(self.STRIPPED) + b"RUNNER_TEMP=/tmp\0")
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("embeds the environment of the job", done.stderr)

    def test_allow_build_env_downgrades_to_a_warning(self):
        done = self.run_gate(
            elf64(self.STRIPPED) + b"RUNNER_TEMP=/tmp\0", "--allow-build-env"
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertIn("::warning::", done.stdout)
        self.assertNotIn("no build environment", done.stdout)

    def test_an_artifact_that_trips_both_rules_is_told_both(self):
        """Exiting on the first failure sent the reader back for a second
        release run to find the other one."""
        done = self.run_gate(elf64(self.UNSTRIPPED) + b"GITHUB_RUN_ID=1\0")
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("ships debug information", done.stderr)
        self.assertIn("embeds the environment of the job", done.stderr)

    def test_no_binary_at_all_exits_nonzero(self):
        done = self.run_gate(b"not a binary, and too short to inspect")
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("read nothing", done.stderr)


class ElfTest(unittest.TestCase):
    def test_stripped_executable_passes(self):
        blob = elf64([(".text", SHT_PROGBITS, SHF_ALLOC | 0x4, 32),
                      (".dynsym", 11, SHF_ALLOC, 24)])
        self.assertEqual(check.check_elf("outl", blob, False), [])

    def test_dwarf_fails(self):
        blob = elf64([(".text", SHT_PROGBITS, SHF_ALLOC | 0x4, 32),
                      (".debug_info", SHT_PROGBITS, 0, 64)])
        failures = check.check_elf("outl", blob, False)
        self.assertEqual(len(failures), 1)
        self.assertIn(".debug_info", failures[0])

    def test_symtab_fails(self):
        blob = elf64([(".symtab", SHT_SYMTAB, 0, 48)])
        failures = check.check_elf("outl", blob, False)
        self.assertEqual(len(failures), 1)
        self.assertIn(".symtab", failures[0])

    def test_symtab_allowed_with_flag(self):
        blob = elf64([(".symtab", SHT_SYMTAB, 0, 48)])
        self.assertEqual(check.check_elf("outl", blob, True), [])

    def test_loaded_gdb_scripts_section_is_not_debug_info(self):
        # rustc writes `.debug_gdb_scripts` into every executable built with
        # debuginfo, flagged `AMS`: allocated, mapped in the read-only
        # segment, and read by `main` so the linker keeps it. `strip` never
        # removes an allocated section, and it holds 34 bytes naming
        # `gdb_load_rust_pretty_printers.py`, not DWARF.
        blob = elf64([(".text", SHT_PROGBITS, SHF_ALLOC | 0x4, 32),
                      (".debug_gdb_scripts", SHT_PROGBITS,
                       SHF_ALLOC | SHF_MERGE_STRINGS, 34)])
        self.assertEqual(check.check_elf("outl", blob, False), [])

    def test_unloaded_gdb_scripts_section_still_fails(self):
        # Without SHF_ALLOC it is a section `strip` would have taken, so its
        # presence still says the strip never ran.
        blob = elf64([(".debug_gdb_scripts", SHT_PROGBITS, SHF_MERGE_STRINGS, 34)])
        self.assertEqual(len(check.check_elf("outl", blob, False)), 1)


class ContainerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def apk(self, members):
        path = os.path.join(self.tmp.name, "outl-android-arm64.apk")
        with zipfile.ZipFile(path, "w") as archive:
            for name, blob in members.items():
                archive.writestr(name, blob)
        return path

    def test_java_class_in_apk_is_not_a_native_binary(self):
        # `DebugProbesKt.bin` from kotlinx-coroutines is a class file that
        # every APK depending on coroutines carries at its root.
        stripped = elf64([(".text", SHT_PROGBITS, SHF_ALLOC | 0x4, 32)])
        path = self.apk({
            "lib/arm64-v8a/liboutl_mobile_lib.so": stripped,
            "DebugProbesKt.bin": java_class(52),
        })
        results = verdict(path)
        self.assertEqual(results["failures"], [])
        self.assertEqual(len(results["checked"]), 1)
        self.assertIn("liboutl_mobile_lib.so", results["checked"][0])

    def test_unstripped_library_in_apk_fails(self):
        unstripped = elf64([(".text", SHT_PROGBITS, SHF_ALLOC | 0x4, 32),
                            (".debug_line", SHT_PROGBITS, 0, 64),
                            (".symtab", SHT_SYMTAB, 0, 48)])
        path = self.apk({"lib/arm64-v8a/liboutl_mobile_lib.so": unstripped})
        self.assertEqual(len(verdict(path)["failures"]), 2)

    def test_apk_with_only_class_files_reads_nothing(self):
        path = self.apk({"DebugProbesKt.bin": java_class(52)})
        self.assertEqual(verdict(path)["checked"], [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
