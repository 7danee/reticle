# Third-party notices

reticle itself is licensed under the reticle License (see `LICENSE`). It contains or is built from the following third-party components, which keep their own licenses.

## Fonts (embedded in `reticle.exe`)

| Component | License |
|---|---|
| Poppins (Regular, SemiBold), Indian Type Foundry | SIL Open Font License 1.1 – https://openfontlicense.org |
| Font Awesome Free 6.5.2 (solid icon font), Fonticons, Inc. | Font: SIL Open Font License 1.1; icons: CC BY 4.0 – https://fontawesome.com/license/free |

## Rust libraries

reticle is compiled from these crates and their dependencies, all under permissive licenses (mostly MIT and/or Apache-2.0; some BSD/Zlib/ISC):

| Crate | Purpose |
|---|---|
| windows (Microsoft) | Win32 API: overlay window, tray, hotkeys |
| tiny-skia | drawing the crosshair |
| resvg | SVG images |
| skrifa | drawing the "r." logo from the font |
| eframe / egui | settings window |
| rfd | file dialogs |
| serde, serde_json | config file |
| miniz_oxide | compressing share codes |

The full dependency list is in `Cargo.lock`; each crate's license text is published with the crate on https://crates.io.

## Formats

- Counter-Strike crosshair codes are decoded following the layout documented by the MIT-licensed project csgo-sharecode (https://github.com/akiver/csgo-sharecode). No code was copied.
- reticle is not affiliated with Valve, Riot Games or the makers of Crosshair X.
