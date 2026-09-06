# Bundled font

`NotoSansSC-Regular.ttf` is an embedded fallback for Simplified Chinese and other glyphs absent from egui's default fonts. It is appended to both proportional and monospace font families, preserving the existing Latin UI appearance.

- Source: [Noto Sans SC in Google Fonts](https://github.com/google/fonts/tree/main/ofl/notosanssc).
- Original file: `NotoSansSC[wght].ttf`.
- Original SHA-256: `a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da`.
- Transformation: a static instance at weight 400, generated with FontTools 4.64.0 `fontTools.varLib.instancer.instantiateVariableFont`; no glyph subsetting.
- Copyright: 2014–2021 Adobe, with Reserved Font Name "Source".
- License: SIL Open Font License 1.1, reproduced in `OFL-NotoSansSC.txt`.

The full character coverage also helps display device and user-defined Windows plan names. The font is embedded at build time and is not installed system-wide. The downloaded variable font and the local conversion tool are build scratch files, not runtime dependencies.
