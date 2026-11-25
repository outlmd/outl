# Capture from your phone without opening outl

The cost of a thought is the time between having it and seeing it written down.
On a phone, launching an app sits in the middle of that.
These gestures add a line to today's journal without making you wait for outl.

Pick the one that matches how you already use your phone:

| You want | iPhone and iPad | Android |
|---|---|---|
| One tap from the Home Screen | Widget | Widget |
| One tap from the Lock Screen | Lock Screen widget | Quick settings tile |
| One tap from Control Center | **Quick Add** control (iOS 18 and later) | Quick settings tile |
| Type a line without leaving another app | Shortcut: "Add to today" | Not available |
| Send text or a link from another app | Share sheet | Share sheet |
| Say it | Siri: "Add to today in outl" | Not available |

Back in the app, the same job is the `+` in the middle of the bottom bar, see [the phone guide](mobile.md#the-screen).

---

## What each gesture does

There are two kinds, and the difference decides what you see.

**Opens the capture sheet.**
The Home Screen widget, the Lock Screen widget, the Control Center control and the Android tile open outl straight onto a small sheet over today's journal, with the keyboard already up.
They write nothing themselves.
Type the line, press Return, and it is in the journal.
The sheet stays open after each line, so several thoughts in a row cost one gesture each.
Tap **Done** to close it.

**Saves a line without opening outl.**
The share sheet and the "Add to today" Shortcut write the line to a small inbox on the phone and say **"Saved, opens in outl."**
The line is not in your journal yet.
outl moves it in the next time it opens or comes back to the foreground.

That wording is the real state, and it is deliberate.
The line is safe on this phone, but it is not yet in the journal and not yet on your other devices.

---

## iPhone and iPad

### Home Screen or Lock Screen widget

1. Touch and hold the Home Screen, tap **+**, search for **outl**, choose **Add to today**.
2. For the Lock Screen: touch and hold the Lock Screen, tap **Customize**, then **Lock Screen**, and add the **Add to today** widget in the circular or rectangular slot.

A tap opens outl on the capture sheet.
The Home Screen widget comes in the small size, the Lock Screen widget in the circular and rectangular sizes.

### Control Center, iOS 18 and later

1. Open Control Center, tap **+** at the top left, choose **Add a Control**.
2. Pick **Quick Add** under outl.

One tap opens outl on the capture sheet.
The same control can sit on the Lock Screen or be assigned to the Action button.
Older iOS versions do not show it.

### The "Add to today" Shortcut

This is the one that works with outl closed.
It asks **"What should go in today's journal?"**, saves your answer to the inbox, and replies "Saved, opens in outl."

- **Siri:** say "Add to today in outl" or "Add a line to outl".
- **Action button:** Settings → Action Button → Shortcut → choose **Add to today**.
- **Back Tap or the Shortcuts app:** add the **Add to today** action from outl to any Shortcut or automation.

### Share sheet

In any app, tap **Share**, choose **outl**.
It accepts text and one web link.
A multi-line selection is joined into one line with spaces, because a capture is always a single block.

---

## Android

### Home screen widget

Touch and hold the home screen, open **Widgets**, find **outl**, drag the **Add to today** widget out.
A tap opens outl on the capture sheet.

### Quick settings tile

Swipe down twice to open quick settings, tap the pencil (edit), and drag the **Add to today** tile into the active area.
A tap opens outl on the capture sheet.
The exact gesture for editing tiles differs a little between Android versions and phone makers.

### Share sheet

In any app, tap **Share**, choose **outl**.
It accepts plain text.
It saves the line and shows **"Saved, opens in outl."**, or **"Nothing to save."** when what you shared was empty.

---

## Where the line lands

- **One line, one block.**
  Newlines in what you send are collapsed to spaces.
- **At the end of the journal for the day you captured it**, not the day outl drained it.
  A line you saved at 23:58 and outl picked up at 00:10 goes into the 23:58 journal, even if that creates a past day's page.
- **In the order you captured them.**
  Against what you typed in outl itself, order follows when outl picked each one up.
- **Never twice.**
  If outl is interrupted while moving a capture in, the next launch finishes it onto the same block.
- **It is an ordinary block afterwards.**
  It syncs, you can undo it, and it carries its capture time.

On iOS outl picks a share-sheet or Shortcut capture up about a second after you save it, when the app is already on screen.
Otherwise it happens on launch and on every return to the foreground.

---

## If nothing shows up

| What you see | Why, and what to do |
|---|---|
| The widget or control opens outl but no sheet appears | They open it through the `outl://capture` link. Open an issue with your OS version and which entry point you used. |
| "Saved, opens in outl." but the line is not in the journal | Open outl. Captures wait on this phone until the app opens. They are not synced from the inbox. |
| A line is missing after you opened outl | A file outl could not read stays in the inbox, and the app reports the count when it drains. Open an issue with the message. |
| Siri does not know "Add to today" | Say the phrase exactly: "Add to today in outl" or "Add a line to outl". Or run it from the Shortcuts app. |

More in [mobile-troubleshooting.md](mobile-troubleshooting.md).

---

## Same idea on desktop and in a terminal

`outl capture "your line"` writes to the same kind of inbox and lands the next time a client opens that workspace.
See [cli.md → Capture](cli.md#capture).
Per-client coverage is the `CaptureOutsideTheApp` row in [`client-parity.md`](client-parity.md), and the design is [RFC 0363](rfcs/0363-ios-quick-capture.md).
