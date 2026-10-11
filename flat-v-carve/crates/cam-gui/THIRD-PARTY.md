# Runtime notices

Sketch solving uses Arael and arael-sketch-solver at revision
`18d86e1fda91814a4a42d00001f3e8461c638bb4` under the MIT license. The unmodified
notice is shipped in `licenses/Arael-MIT.md` for native and browser packages.

The application uses egui/eframe/egui-wgpu 0.33.3 and wgpu 27.0.1, rfd,
wasm-bindgen, serde/serde_json and bytemuck. Exact dependency versions are in the
workspace Cargo.lock. The framework qualification inventory remains historical
evidence in `experiments/gui1`; it is not the production dependency inventory.

The unmodified epaint_default_fonts 0.33.3 font notices are included in `licenses`:
Hack (MIT and inherited Bitstream Vera terms), Noto Emoji (SIL OFL), Ubuntu Light
(Ubuntu Font Licence), and emoji-icon-font (MIT). Packaged review builds include
these notices. Full target-specific dependency notice assembly remains part of
release qualification.
