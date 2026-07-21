# Freally MIDI Master — End User License Agreement (EULA)

**Software:** Freally MIDI Master ("the Software")
**Licensor:** Mike Weaver <mythodikalone@gmail.com>, trading as Havoc Software ("the Licensor")
**Effective date:** ______ *(to be set on release)*

By installing, copying, or using the Software, you ("the User") agree to this
Agreement. If you do not agree, do not install or use the Software.

## 1. License grant
The Software is **proprietary** and **All Rights Reserved**. Subject to this
Agreement, the Licensor grants you a personal, non-exclusive, non-transferable,
revocable license to install and use the Software for your own lawful purposes.
No ownership rights are transferred. You may not sell, sublicense, rent, lease, or
redistribute the Software.

The Software is provided **free of charge, in full** — every feature, for
everyone. There is no paid tier, no license key, no in-app purchase, and no
advertising.

## 2. Restrictions
Except to the extent applicable law expressly permits, or as expressly permitted by
the contribution terms in `LICENSE`, you may not: (a) reverse engineer, decompile, or
disassemble the Software; (b) remove or alter any proprietary notices; (c) copy the
Software to develop a competing product; or (d) use the Software in violation of any
law.

## 3. Your output, and your responsibility
The Software is a general-purpose tool that procedurally generates original MIDI
patterns, arrangements, and rendered audio, and that plays back audio samples you
choose to import ("User Content").

**The MIDI, audio, and project files you generate with the Software are yours.** The
Licensor claims no ownership of them and asserts no license over them. You may use
them for any lawful purpose, including commercial release, with no royalty and no
attribution requirement.

**You are solely responsible for your User Content and for how you use the
Software,** including ensuring that you have all necessary rights and permissions and
that your use complies with all applicable laws — including, without limitation:

- **Copyright and intellectual-property** laws — in particular, you are responsible
  for the one-shots, samples, and audio files you import into the Software. Do not
  import or distribute audio you do not have the right to use.
- **Trademark and publicity** laws — artist and producer names appear in the Software
  solely as descriptive references to a musical style. They do not imply endorsement,
  affiliation, or authorship, and you should not market your output as being *by* an
  artist, or as an official or authorised work of theirs. See
  `docs/legal/disclaimer.md`.
- **Defamation and anti-harassment** laws.

The Software's style dataset encodes **research-derived stylistic parameters**
(tempo ranges, swing values, note-density and contour rules, arrangement
conventions). It contains no copied MIDI, no sampled audio, and no reproduction of
any specific recording, and the Software has no feature that recreates a specific
song. Musical style, as distinct from a specific fixed recording or composition, is
not itself protected by copyright — but you remain responsible for the finished work
you release.

**The Licensor is not responsible for, and assumes no liability for, any User Content
you create, generate, import, export, share, or distribute, or for any use you make
of the Software.**

## 4. Indemnification
You agree to indemnify, defend, and hold harmless the Licensor from and against
any claims, damages, liabilities, losses, and expenses (including reasonable legal
fees) arising out of or related to your User Content, your use of the Software, or
your breach of this Agreement.

## 5. Architecture, network use, and third-party components
The Software requires **no account**, collects **no telemetry**, and never sends your
projects, your generated output, your imported audio, or any information about you or
your machine anywhere. Music generation, playback, and export are entirely local and
never touch the network.

There are exactly two outbound connections, both described here so this document
stays accurate:

- **Update check.** Once per launch, and again whenever you choose *Check for
  updates…*, the Software fetches one small file from this project's GitHub releases
  page in order to compare version numbers. Nothing about you, your machine, or your
  content is transmitted. If a newer version exists you are shown its version number
  and release notes and asked **yes or no** — nothing downloads or installs without
  that answer. Updates are cryptographically signature-verified before installation.
  If you are offline, rate-limited, or already current, the Software stays silent.
- **Crash reports.** These are **never transmitted automatically**. A report is
  written to a local file, shown to you in full, and sent only if you choose to send
  it — from your own email client or your own GitHub account. See
  `docs/legal/disclaimer.md`.

Both can be reviewed in the source; neither is on a server the Licensor operates.

The Software contains **no machine-learning or artificial-intelligence components of
any kind**. It downloads no models and performs no inference. All generation is
deterministic, rule-based, procedural code operating on hand-authored style data;
the same seed and settings reproduce the same output.

The Software incorporates third-party open-source components under their own
licenses (listed in `THIRD-PARTY-NOTICES.md`), and bundles fonts under the SIL Open
Font License. The preview instrument kits are synthesized by the Licensor's own
tooling and contain no third-party samples. Crash reports are **never transmitted
automatically**: a report is written locally, shown to you in full, and sent only if
you choose to send it (see `docs/legal/disclaimer.md` and the in-app report dialog).

## 6. No warranty
THE SOFTWARE IS PROVIDED "AS IS" AND "AS AVAILABLE", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE IMPLIED WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, AND NON-INFRINGEMENT. THE
LICENSOR DOES NOT WARRANT THAT THE SOFTWARE WILL BE ERROR-FREE OR UNINTERRUPTED, OR
THAT GENERATED OUTPUT WILL SUIT ANY PARTICULAR PURPOSE, OR THAT AUDIO PLAYBACK WILL
BE FREE OF DROPOUTS OR LATENCY, WHICH MAY DEPEND ON YOUR HARDWARE, DRIVERS, AND
AUDIO DEVICE CONFIGURATION.

## 7. Limitation of liability
TO THE MAXIMUM EXTENT PERMITTED BY LAW, THE LICENSOR WILL NOT BE LIABLE FOR ANY
INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL, OR PUNITIVE DAMAGES, OR FOR ANY LOSS
OF DATA, PROJECTS, SESSIONS, PROFITS, OR GOODWILL, ARISING OUT OF OR RELATED TO THE
SOFTWARE OR THIS AGREEMENT, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGES. THE
LICENSOR'S TOTAL LIABILITY WILL NOT EXCEED THE AMOUNT YOU PAID FOR THE SOFTWARE
(WHICH IS ZERO — THE SOFTWARE IS FREE OF CHARGE).

## 8. Termination
This license terminates automatically if you breach it. On termination you must
stop using and delete the Software. Sections 3–7 survive termination.

## 9. Governing law
This Agreement is governed by the laws of ______ *(the Licensor's jurisdiction,
to be completed on legal review)*, without regard to conflict-of-laws rules.

## 10. Entire agreement
This Agreement, together with `LICENSE` and `THIRD-PARTY-NOTICES.md`, is the
entire agreement between you and the Licensor regarding the Software.

© Mike Weaver <mythodikalone@gmail.com> — All Rights Reserved.
