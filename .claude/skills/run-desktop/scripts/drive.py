"""Drive the outl desktop window: real CGEvents + screencapture.

macOS WKWebView, so no CDP and no Playwright. Input goes through
Quartz CGEvents (keyboard + mouse), observation through screencapture.
Coordinates are in POINTS, the same space screencapture -R uses.
"""
import subprocess, sys, time
import Quartz as Q

KEYCODE = {
    "return": 36, "enter": 36, "tab": 48, "space": 49, "escape": 53,
    "left": 123, "right": 124, "down": 125, "up": 126,
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7,
    "c": 8, "v": 9, "b": 11, "q": 12, "w": 13, "e": 14, "r": 15,
    "y": 16, "t": 17, "o": 31, "u": 32, "i": 34, "p": 35, "l": 37,
    "j": 38, "k": 40, "n": 45, "m": 46,
    "1": 18, "2": 19, "3": 20, "4": 21, "5": 23, "6": 22, "7": 26,
    "8": 28, "9": 25, "0": 29,
    "backspace": 51, "delete": 51,
}
MODS = {
    "cmd": Q.kCGEventFlagMaskCommand, "shift": Q.kCGEventFlagMaskShift,
    "alt": Q.kCGEventFlagMaskAlternate, "ctrl": Q.kCGEventFlagMaskControl,
}

def _post(ev):
    Q.CGEventPost(Q.kCGHIDEventTap, ev)

# Virtual keycodes for the modifiers themselves. A chord has to press
# and RELEASE these for real.
MOD_KEYS = {"cmd": 55, "shift": 56, "alt": 58, "ctrl": 59}


def key(name, *mods, delay=0.06):
    """Send one chord, and leave no modifier behind.

    Setting the flag on the key event alone is not enough, and the way
    it fails is expensive: the flag rides along into whatever is sent
    next. A `Cmd+P` followed by typing `meet` turned the `m` into
    `Cmd+M`, which on macOS **minimises the window** — mid-test, on a
    real machine, with the app then unreachable.

    So the modifier is pressed and released as its own key around the
    chord, and every plain key explicitly clears its flags.
    """
    code = KEYCODE[name.lower()]
    flags = 0
    for m in mods:
        flags |= MODS[m]
    for m in mods:
        _post(Q.CGEventCreateKeyboardEvent(None, MOD_KEYS[m], True))
        time.sleep(0.01)
    for down in (True, False):
        ev = Q.CGEventCreateKeyboardEvent(None, code, down)
        Q.CGEventSetFlags(ev, flags)
        _post(ev)
        time.sleep(0.012)
    for m in reversed(mods):
        _post(Q.CGEventCreateKeyboardEvent(None, MOD_KEYS[m], False))
        time.sleep(0.01)
    time.sleep(delay)

def text(s, delay=0.03):
    """Type a string.

    WKWebView ignores a CGEvent carrying only a unicode string on
    keycode 0 — the picker stayed empty through a whole typed word.
    So every character that has a real keycode is sent as that key,
    and only the rest falls back to the unicode payload.
    """
    shifted = {"!": "1", "@": "2", "#": "3", "$": "4", "%": "5",
               "^": "6", "&": "7", "*": "8", "(": "9", ")": "0",
               "?": "/", ":": ";", '"': "'", "<": ",", ">": ".", "_": "-"}
    extra = {"-": 27, "=": 24, "[": 33, "]": 30, ";": 41, "'": 39,
             ",": 43, ".": 47, "/": 44, "`": 50, " ": 49}
    for ch in s:
        low = ch.lower()
        mods = ("shift",) if ch.isupper() or ch in shifted else ()
        base = shifted.get(ch, low)
        code = KEYCODE.get(base, extra.get(base))
        if code is not None:
            key(base if base in KEYCODE else base, *mods, delay=delay) \
                if base in KEYCODE else _raw(code, mods, delay)
            continue
        for down in (True, False):
            ev = Q.CGEventCreateKeyboardEvent(None, 0, down)
            Q.CGEventSetFlags(ev, 0)
            Q.CGEventKeyboardSetUnicodeString(ev, len(ch), ch)
            _post(ev)
            time.sleep(0.008)
        time.sleep(delay)


def _raw(code, mods, delay):
    flags = 0
    for m in mods:
        flags |= MODS[m]
    for down in (True, False):
        ev = Q.CGEventCreateKeyboardEvent(None, code, down)
        if flags:
            Q.CGEventSetFlags(ev, flags)
        _post(ev)
        time.sleep(0.012)
    time.sleep(delay)

def move(x, y, delay=0.12):
    ev = Q.CGEventCreateMouseEvent(None, Q.kCGEventMouseMoved, (x, y), 0)
    _post(ev)
    time.sleep(delay)

def click(x, y, delay=0.15):
    move(x, y, 0.05)
    for t in (Q.kCGEventLeftMouseDown, Q.kCGEventLeftMouseUp):
        ev = Q.CGEventCreateMouseEvent(None, t, (x, y), Q.kCGMouseButtonLeft)
        _post(ev)
        time.sleep(0.02)
    time.sleep(delay)

def scroll(dy, x=None, y=None, delay=0.15):
    if x is not None:
        move(x, y, 0.05)
    ev = Q.CGEventCreateScrollWheelEvent(None, Q.kCGScrollEventUnitLine, 1, dy)
    _post(ev)
    time.sleep(delay)

def shot(path, region=None):
    cmd = ["screencapture", "-x", "-t", "png"]
    if region:
        cmd += ["-R", ",".join(str(int(v)) for v in region)]
    cmd.append(path)
    subprocess.run(cmd, check=True)
    return path

def focused_window():
    """The id of the window that currently owns the keyboard."""
    out = subprocess.run(["aerospace", "list-windows", "--focused"],
                         capture_output=True, text=True).stdout.strip()
    return out.split("|")[0].strip() if out else None


def focus(window_id):
    subprocess.run(["aerospace", "focus", "--window-id", window_id],
                   capture_output=True)
    time.sleep(0.4)


def require_focus(window_id):
    """Refuse to send anything unless that window owns the keyboard.

    Targeting by id is not the same as holding focus. A window can be
    closed, moved to another workspace, or simply lose focus between
    two calls, and synthetic keys then land in whatever is in front.
    That is not hypothetical: an `Escape` and two typed words went into
    a different terminal session this way, interrupting work that had
    nothing to do with the test.

    Call this immediately before every key, tap and type — not once at
    the start.
    """
    if focused_window() != window_id:
        focus(window_id)
    got = focused_window()
    if got != window_id:
        raise RuntimeError(
            f"focus is on window {got}, not {window_id}; nothing was sent"
        )
