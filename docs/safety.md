# Safe mode

## Why

One X7 stopped answering after the very first command this software
sent it. The sequence, from a usbmon capture and the kernel log:

1. Volume changes through ALSA (UAC `SET_CUR` on feature unit 0x14):
   answered normally.
2. Three short recordings from the X7 ("What U Hear", 48 kHz, 16 bit).
   During each, the xHCI driver logged `buffer overrun event` for the
   X7's capture endpoint. Control requests afterwards were still
   answered.
3. About four minutes later, `x7 output`: one HID `SET_REPORT`
   (`21 09 0200 0004`, 64 bytes, payload `23 4d`, a read). The control
   transfer never completed; the kernel gave up after 5 s and `write()`
   returned `ETIMEDOUT`.
4. From then on the device answered no control request at all: the
   sample-rate request of the next playback failed (`-110`, then
   `-71`), re-authorizing the device failed (`can't set config`), and a
   USB port reset ended in "device not accepting address". Only cutting
   the X7's power brought it back.

The firmware version of that X7 is unknown. Whether the recordings
contributed is open. The same command works on other X7s.

So this software now assumes that any command can wedge the device, and
that anything sent to a wedged X7 makes things worse.

## Rules

Enforced in `x7/src/transport.rs` (every client goes through it) and in
the GUI session (`x7/src/session.rs`):

- **No command while the X7 records.** Before every write the
  transport reads `/proc/asound/cardN/pcm*c/sub*/status` of the X7's
  own ALSA card. An open capture stream, or no ALSA card bound at all,
  means the command is not sent (`CaptureOpen`). The CLI exits with
  that message; the GUI keeps the command and sends it once the
  recording ends.
- **Reads first.** A write (anything that is not one of the known reads
  in `protocol::is_read_command`) goes out only after a read was
  answered on the same handle. The CLI sends `23 4d` first by itself.
- **A hang stops everything, once.** No reply within 500 ms (with no
  other protocol report in between), a failed `write()`, or a write
  slower than 1 s: the transport stops sending on this handle, writes a
  marker (`$XDG_RUNTIME_DIR/x7/hang`, else `~/.cache/x7/hang`) holding
  the USB bus and device number and the boot id, and reports
  "power-cycle the X7". There is no retry, no resync command, no USB
  reset, no re-authorization.
- **No reopen of the same enumeration.** `Device::open` refuses while
  the marker names the present device; the GUI also remembers it in
  memory. Power-cycling re-enumerates the X7 with a new device number,
  and the marker clears itself on the next open.
- **A rejection is not a hang.** If the device answers with some other
  protocol report (for example the echo of a different output), it is
  alive: the command is reported as not applied and nothing trips.
- **Quiet start.** The GUI waits 3 s after opening before its first
  read, because the firmware answers only about two seconds after
  enumerating and a missed read now counts as a hang.
- **Unplugged is not hung.** `ENODEV` or EOF means the node is gone;
  the GUI shows "not connected" and opens the next enumeration.

## Log

Every open, check, command (`tx read` / `tx write`), report (`rx`),
refusal and hang is one line in `$X7_LOG`, else
`$XDG_STATE_HOME/x7/x7.log`, else `~/.local/state/x7/x7.log`, UTC, with
the process id. The file is rotated to `x7.log.1` at 1 MiB.

`x7 status` shows the state without sending anything: device node,
access, USB bus and device, open recordings, hang marker, log path.

## Tests

`transport::tests` replays the hang with a stand-in for the hidraw node
(`HungX7`: the first `write()` stalls and fails with `ETIMEDOUT`, every
later one with `EPROTO`) and checks that exactly one write ever reaches
it. Further tests cover a silent device, a slow write, an open
recording, a missing ALSA card, writes before a read, an unplugged
device, and in `session::tests` the GUI path: no retry, no reopen of
the same enumeration, reopen after re-enumeration, the settle time,
the pause during a recording, and rejection without a hang.
