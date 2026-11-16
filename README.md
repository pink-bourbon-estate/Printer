# Printer

Local print and scan for an IPP/eSCL printer (built for an HP Envy 6432, but anything IPP Everywhere / AirScan works), from one small Rust binary.

No vendor app, no account, no sign-in, no subscription check, no internet connection. The only network address this program ever contacts is your printer's own IP, on your own LAN.

## Why

Vendor printer apps (HP Smart and friends) tend to need an account, a subscription, and a working internet connection just to print a page you already own the ink for — and to lose track of the printer every other week. This talks to the printer directly, using the open protocols it already speaks in its own firmware: IPP for printing, eSCL for scanning. Those are the same protocols AirPrint and Mopria use.

## What this does and doesn't change

| | |
|---|---|
| App can't find the printer | Found once (mDNS, or a direct network sweep), then remembered in a config file. |
| "You must be signed in to print" | There's no login anywhere in this program. |
| "You need a subscription to print" | There's no subscription check anywhere in this program. |
| App refuses because ink reads low | **This never happens here.** Ink levels are read from the printer and shown; they never stop a job. See `printjob.rs` — there is no code path that returns early because of a supply level. |
| Cartridge authentication (HP's Dynamic Security, HP+) | **Out of reach of any host program**, this one included. That check runs inside the printer's own firmware, between the firmware and the chip on the cartridge, before your computer is involved. No software running on your PC — not this, not anything claiming otherwise — can overrule a decision made entirely inside the printer. |

If your printer is HP+-enabled, see the note at the bottom.

## Install

```
cargo install --path .
```

or just `cargo build --release` and use `target/release/printer`.

## Use

```
printer                                   printer status (the default)
printer find                              find printers on the network
printer --host 192.168.1.50 status        state, ink, capabilities
printer scan                              scan one page from the flatbed
printer scan --feeder --dpi 600           scan every page in the feeder
printer print report.pdf
printer print photo.jpg --copies 2 --gray
printer jobs                              what the printer is doing
printer cancel 42
```

The printer is remembered after the first `find` or `--host`, in `~/.config/printer-rs/printer.tsv`.

## How it works

| Job | Standard | Why |
|---|---|---|
| Printing | IPP (RFC 8010 / 8011), IPP Everywhere | What AirPrint and Mopria use; implemented in the printer's own firmware. |
| Page format | PDF passthrough, or generated PWG Raster / JPEG | The printer is asked what it accepts (`document-format-supported`) and the best match is sent — nothing assumed. |
| Scanning | eSCL / AirScan | The protocol Apple published; HP, Canon, Epson and Brother all implement it. |
| Finding it | mDNS (`_ipp._tcp`, `_uscan._tcp`), or a subnet sweep | mDNS gives the real port and resource path rather than a guessed one. |

Written from the public specifications (RFC 8010/8011, PWG 5102.4, the eSCL spec), not from a decompiled driver or HP SDK. No PDF rasteriser is bundled: a PDF prints untouched on a printer that accepts `application/pdf`; on a printer that doesn't, this build says so honestly rather than silently mangling the page.

## Tests

```
cargo test
```

29 tests: the IPP binary codec round-tripped against hand-built wire bytes, the PWG Raster header and run-length compression verified bit-exact (including the 1796-byte header's documented field offsets), the eSCL settings XML checked for schema-correct element order, and the print pipeline run end-to-end against a small fake IPP server — including one test that a job succeeds while ink reads 0%, because the printer deciding that is not this program's job.

## Privacy

This program does not collect anything about you and talks to nothing but your printer. No accounts, no analytics, no telemetry, no cloud. Where it needs the network — printing, scanning, and finding the printer via mDNS or a local subnet sweep — it says so above, and it stays on your own LAN.

## Licence

MIT. Use, copy, change, share and fork freely, keeping the copyright notice. A request rather than a condition: please don't resell this or build a paid product out of it.

## If your printer has HP+ active

Cartridge authentication is enforced entirely inside the printer, so no program on your computer — including this one — can make an HP+ printer accept a rejected cartridge. What's documented, for what it's worth:

- HP's own stated position is that cartridges carrying a genuine HP chip keep working through Dynamic Security firmware updates, even when refilled — it's specifically non-HP chips that get blocked. A refilled *genuine* cartridge is the most likely route to cheap ink that keeps working.
- HP+ activation is reported as permanent and not user-removable from the printer's own settings; asking HP support to deactivate it is the one lever outside the firmware itself, and it isn't guaranteed to work.
- An HP+ printer needs to stay online to print at all — don't block its internet access at your router, unlike a non-HP+ printer, where that's a reasonable way to stop it phoning home.

None of that is a limitation of this program specifically; it's true of every piece of software that isn't the printer's own firmware.
