# UI asset manifest

The application uses original vector line art in `flat-v-carve/crates/cam-gui/src/ui_icons.rs`. The shapes are authored in this repository, with no downloaded artwork, font glyphs, raster dependencies or additional license obligations. They follow the repository's licensing.

Icons use a 20-point drawing grid, 18–20-point visible size, 1.5-point strokes, and at least a 28-point button target. Text labels, accessible widget names and hover explanations accompany actions. Color comes from the semantic theme rather than being baked into assets. The same painter code runs on native and WASM.

| Family | Shapes |
|---|---|
| Navigation | Stock, Machine, Library, Settings, Artwork |
| Artwork state | Eye, Hidden, Lock, Unlock |
| Operations and cutters | Face, Carve, Profile, Knife, Endmill, V-bit |
| Actions | Add, More, Save, Undo, Redo |
| View and playback | Fit, Play, Pause, Start |
| Assistance | Warning, Help |

`ui_widgets::stock_datum` is a procedural schematic with live thickness, clearance-above-stock and selected Z0. It is not drawn to scale. Unset values remain explicit; labels use the committed setup values. Partial input is retained by the existing editor and must be completed before generation. It does not depict machine touch-off or claim to validate a machine setup.

Additional cutter/operation diagrams should follow the same approach when dimensions and state matter. Generated bitmap concepts may guide composition, but no text-bearing concept raster is shipped as an interactive panel. Decorative stock textures and a font replacement remain optional and outside the current milestone.

`tool_diagram::show` draws the Library's live endmill, truncated V-bit and knife schematics from parsed draft geometry. Milling outlines preserve the diameter/length proportions; the V-bit taper uses its declared angle and tip diameter, capped at its cutting height. A shaft is drawn only when both diameter and extent are known; no holder or flute shape is invented. Knife diagrams show pivot-to-tip offset and declared cut depth. Dimensions remain text. Invalid dimensions produce an explicit incomplete-geometry state. The diagram is original egui painter code under the repository's license, shares the semantic palette, and performs no image decoding or remote loading.
