# Third-party licenses

Piuma itself is licensed under Apache-2.0 (see `LICENSE`). This file lists
the third-party crates compiled or used at build time for the Linux binaries
(`piuma` and `piuma-setup`), as resolved by `Cargo.lock`.

Generated from the current lock file with:

```sh
cargo tree -e normal,build --target x86_64-unknown-linux-gnu --prefix none --format '{p}|{l}' | sort -u
```

Packages in the Linux build: **399**. Licenses in use:

- `MIT OR Apache-2.0` — 150
- `MIT` — 109
- `Apache-2.0 OR MIT` — 41
- `Unicode-3.0` — 18
- `MIT/Apache-2.0` — 17
- `Apache-2.0` — 12
- `MIT OR Apache-2.0 OR Zlib` — 11
- `BSD-2-Clause` — 6
- `BSD-3-Clause` — 6
- `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` — 4
- `Apache-2.0/MIT` — 3
- `Zlib` — 3
- `Zlib OR Apache-2.0 OR MIT` — 3
- `BSD-2-Clause OR Apache-2.0 OR MIT` — 2
- `BSD-3-Clause OR Apache-2.0` — 2
- `MIT OR Zlib OR Apache-2.0` — 2
- `Unlicense OR MIT` — 2
- `(MIT OR Apache-2.0) AND Unicode-3.0` — 1
- `0BSD OR MIT OR Apache-2.0` — 1
- `Apache-2.0 AND MIT` — 1
- `Apache-2.0 OR GPL-2.0-only` — 1
- `CC0-1.0` — 1
- `CC0-1.0 OR Apache-2.0` — 1
- `ISC` — 1
- `MPL-2.0` — 1

## Mozilla Public License 2.0

One crate in the build is licensed under MPL-2.0:

- **`option-ext 0.2.0`** — reached through `freedesktop-icons → dirs → dirs-sys`.
  It is used unmodified; the full license text is in `LICENSES/MPL-2.0.txt`,
  and its source is available at <https://crates.io/crates/option-ext/0.2.0>.
  MPL-2.0 is a file-level copyleft license: it allows commercial and
  proprietary use of the larger work, and its obligations apply only to the
  MPL-covered files (which this project does not modify).

All other crates are under permissive licenses (MIT, Apache-2.0, BSD, ISC,
Zlib, Unicode-3.0, CC0-1.0, BSL-1.0, Unlicense or a combination of these).

## Direct dependencies

| Crate | License |
|---|---|
| `iced` | `MIT` |
| `iced_runtime` | `MIT` |
| `iced_exwlshell (git)` | `MIT` |
| `image` | `MIT OR Apache-2.0` |
| `smol` | `Apache-2.0 OR MIT` |
| `log` | `MIT OR Apache-2.0` |
| `serde` | `MIT OR Apache-2.0` |
| `serde_json` | `MIT OR Apache-2.0` |
| `signal-hook` | `Apache-2.0/MIT` |
| `zbus` | `MIT` |
| `freedesktop-icons` | `MIT` |
| `cosmic-text` | `MIT OR Apache-2.0` |
| `libc` | `MIT OR Apache-2.0` |

## Full list

| Crate | License |
|---|---|
| `ab_glyph v0.2.32` | `Apache-2.0` |
| `ab_glyph_rasterizer v0.1.10` | `Apache-2.0` |
| `adler2 v2.0.1` | `0BSD OR MIT OR Apache-2.0` |
| `ahash v0.8.12` | `MIT OR Apache-2.0` |
| `aligned v0.4.3` | `MIT OR Apache-2.0` |
| `aligned-vec v0.6.4` | `MIT` |
| `android-build v0.1.4` | `MIT` |
| `anyhow v1.0.104` | `MIT OR Apache-2.0` |
| `arg_enum_proc_macro v0.3.4 (proc-macro)` | `MIT` |
| `arrayref v0.3.9` | `BSD-2-Clause` |
| `arrayvec v0.7.8` | `MIT OR Apache-2.0` |
| `as-raw-xcb-connection v1.0.1` | `MIT OR Apache-2.0` |
| `as-slice v0.2.1` | `MIT OR Apache-2.0` |
| `ash v0.38.0+1.3.281` | `MIT OR Apache-2.0` |
| `async-broadcast v0.7.2` | `MIT OR Apache-2.0` |
| `async-channel v2.5.0` | `Apache-2.0 OR MIT` |
| `async-executor v1.14.0` | `Apache-2.0 OR MIT` |
| `async-fs v2.2.0` | `Apache-2.0 OR MIT` |
| `async-io v2.6.0` | `Apache-2.0 OR MIT` |
| `async-lock v3.4.2` | `Apache-2.0 OR MIT` |
| `async-net v2.0.0` | `Apache-2.0 OR MIT` |
| `async-process v2.5.0` | `Apache-2.0 OR MIT` |
| `async-recursion v1.1.1 (proc-macro)` | `MIT OR Apache-2.0` |
| `async-signal v0.2.14` | `Apache-2.0 OR MIT` |
| `async-task v4.7.1` | `Apache-2.0 OR MIT` |
| `async-trait v0.1.92 (proc-macro)` | `MIT OR Apache-2.0` |
| `atomic-waker v1.1.2` | `Apache-2.0 OR MIT` |
| `autocfg v1.5.1` | `Apache-2.0 OR MIT` |
| `av-scenechange v0.14.1` | `MIT` |
| `av1-grain v0.2.5` | `BSD-2-Clause` |
| `avif-serialize v0.8.9` | `BSD-3-Clause` |
| `base64 v0.22.1` | `MIT OR Apache-2.0` |
| `bit-set v0.8.0` | `Apache-2.0 OR MIT` |
| `bit-vec v0.8.0` | `Apache-2.0 OR MIT` |
| `bit_field v0.10.3` | `Apache-2.0/MIT` |
| `bitflags v1.3.2` | `MIT/Apache-2.0` |
| `bitflags v2.13.2` | `MIT OR Apache-2.0` |
| `bitstream-io v4.10.0` | `MIT/Apache-2.0` |
| `blocking v1.7.0` | `Apache-2.0 OR MIT` |
| `built v0.8.1` | `MIT` |
| `bytemuck v1.25.2` | `Zlib OR Apache-2.0 OR MIT` |
| `bytemuck_derive v1.12.1 (proc-macro)` | `Zlib OR Apache-2.0 OR MIT` |
| `byteorder-lite v0.1.0` | `Unlicense OR MIT` |
| `bytes v1.12.1` | `MIT` |
| `calloop v0.13.0` | `MIT` |
| `calloop v0.14.4` | `MIT` |
| `calloop-wayland-source v0.3.0` | `MIT` |
| `calloop-wayland-source v0.4.1` | `MIT` |
| `cc v1.5.1` | `MIT OR Apache-2.0` |
| `cfg-if v1.0.5` | `MIT OR Apache-2.0` |
| `cfg_aliases v0.2.2` | `MIT` |
| `clipboard_wayland v0.2.2` | `Apache-2.0` |
| `clipboard_x11 v0.4.3` | `MIT` |
| `codespan-reporting v0.12.0` | `Apache-2.0` |
| `color_quant v1.1.0` | `MIT` |
| `concurrent-queue v2.5.0` | `Apache-2.0 OR MIT` |
| `core_maths v0.1.1` | `MIT` |
| `cosmic-text v0.15.0` | `MIT OR Apache-2.0` |
| `crc32fast v1.5.2` | `MIT OR Apache-2.0` |
| `crossbeam-deque v0.8.8` | `MIT OR Apache-2.0` |
| `crossbeam-epoch v0.9.21` | `MIT OR Apache-2.0` |
| `crossbeam-utils v0.8.23` | `MIT OR Apache-2.0` |
| `cryoglyph v0.1.0` | `MIT OR Apache-2.0 OR Zlib` |
| `ctor v0.10.1` | `Apache-2.0 OR MIT` |
| `cursor-icon v1.2.0` | `MIT OR Apache-2.0 OR Zlib` |
| `darling v0.24.1` | `MIT` |
| `darling_core v0.24.1` | `MIT` |
| `darling_macro v0.24.1 (proc-macro)` | `MIT` |
| `data-url v0.3.2` | `MIT OR Apache-2.0` |
| `dirs v5.0.1` | `MIT OR Apache-2.0` |
| `dirs-sys v0.4.1` | `MIT OR Apache-2.0` |
| `displaydoc v0.2.7 (proc-macro)` | `MIT OR Apache-2.0` |
| `dlib v0.5.3` | `MIT` |
| `document-features v0.2.12 (proc-macro)` | `MIT OR Apache-2.0` |
| `downcast-rs v1.2.1` | `MIT/Apache-2.0` |
| `dpi v0.1.2` | `Apache-2.0 AND MIT` |
| `either v1.18.0` | `MIT OR Apache-2.0` |
| `endi v1.1.1` | `MIT` |
| `enumflags2 v0.7.12` | `MIT OR Apache-2.0` |
| `enumflags2_derive v0.7.12 (proc-macro)` | `MIT OR Apache-2.0` |
| `equator v0.4.2` | `MIT` |
| `equator-macro v0.4.2 (proc-macro)` | `MIT` |
| `equivalent v1.0.2` | `Apache-2.0 OR MIT` |
| `errno v0.3.14` | `MIT OR Apache-2.0` |
| `etagere v0.2.15` | `MIT/Apache-2.0` |
| `euclid v0.22.14` | `MIT OR Apache-2.0` |
| `event-listener v5.4.2` | `Apache-2.0 OR MIT` |
| `event-listener-strategy v0.5.4` | `Apache-2.0 OR MIT` |
| `exr v1.74.2` | `BSD-3-Clause` |
| `exwlshellev v0.21.0-dev (https://github.com/waycrate/exwlshelleventloop.git?branch=master#014ad7d6)` | `MIT` |
| `fastrand v2.5.0` | `Apache-2.0 OR MIT` |
| `fax v0.2.7` | `MIT` |
| `fdeflate v0.3.7` | `MIT OR Apache-2.0` |
| `find-msvc-tools v0.1.14` | `MIT OR Apache-2.0` |
| `flate2 v1.1.10` | `MIT OR Apache-2.0` |
| `float-cmp v0.9.0` | `MIT` |
| `float_next_after v1.0.0` | `MIT` |
| `foldhash v0.1.5` | `Zlib` |
| `foldhash v0.2.0` | `Zlib` |
| `font-types v0.10.1` | `MIT OR Apache-2.0` |
| `font-types v0.12.5` | `MIT OR Apache-2.0` |
| `fontconfig-parser v0.5.8` | `MIT` |
| `fontdb v0.23.0` | `MIT` |
| `form_urlencoded v1.2.2` | `MIT OR Apache-2.0` |
| `freedesktop-icons v0.4.0` | `MIT` |
| `futures v0.3.34` | `MIT OR Apache-2.0` |
| `futures-channel v0.3.34` | `MIT OR Apache-2.0` |
| `futures-core v0.3.34` | `MIT OR Apache-2.0` |
| `futures-executor v0.3.34` | `MIT OR Apache-2.0` |
| `futures-io v0.3.34` | `MIT OR Apache-2.0` |
| `futures-lite v2.6.1` | `Apache-2.0 OR MIT` |
| `futures-macro v0.3.34 (proc-macro)` | `MIT OR Apache-2.0` |
| `futures-sink v0.3.34` | `MIT OR Apache-2.0` |
| `futures-task v0.3.34` | `MIT OR Apache-2.0` |
| `futures-util v0.3.34` | `MIT OR Apache-2.0` |
| `gethostname v1.1.0` | `Apache-2.0` |
| `getrandom v0.3.4` | `MIT OR Apache-2.0` |
| `getrandom v0.4.3` | `MIT OR Apache-2.0` |
| `gif v0.13.3` | `MIT OR Apache-2.0` |
| `gif v0.14.2` | `MIT OR Apache-2.0` |
| `glam v0.25.0` | `MIT OR Apache-2.0` |
| `glow v0.16.0` | `MIT OR Apache-2.0 OR Zlib` |
| `gpu-alloc v0.6.2` | `MIT OR Apache-2.0` |
| `gpu-alloc-types v0.3.1` | `MIT OR Apache-2.0` |
| `gpu-descriptor v0.3.2` | `MIT OR Apache-2.0` |
| `gpu-descriptor-types v0.2.0` | `MIT OR Apache-2.0` |
| `guillotiere v0.6.2` | `MIT/Apache-2.0` |
| `half v2.7.1` | `MIT OR Apache-2.0` |
| `harfrust v0.3.2` | `MIT` |
| `hashbrown v0.15.5` | `MIT OR Apache-2.0` |
| `hashbrown v0.16.1` | `MIT OR Apache-2.0` |
| `hashbrown v0.17.1` | `MIT OR Apache-2.0` |
| `hex v0.4.3` | `MIT OR Apache-2.0` |
| `hexf-parse v0.2.1` | `CC0-1.0` |
| `iced v0.14.0` | `MIT` |
| `iced_core v0.14.0` | `MIT` |
| `iced_debug v0.14.0` | `MIT` |
| `iced_devtools v0.14.0` | `MIT` |
| `iced_exdevtools v0.21.0-dev (https://github.com/waycrate/exwlshelleventloop.git?branch=master#014ad7d6)` | `MIT` |
| `iced_exwlshell v0.21.0-dev (https://github.com/waycrate/exwlshelleventloop.git?branch=master#014ad7d6)` | `MIT` |
| `iced_exwlshell_macros v0.21.0-dev (proc-macro) (https://github.com/waycrate/exwlshelleventloop.git?branch=master#014ad7d6)` | `MIT` |
| `iced_futures v0.14.0` | `MIT` |
| `iced_graphics v0.14.0` | `MIT` |
| `iced_program v0.14.0` | `MIT` |
| `iced_renderer v0.14.0` | `MIT` |
| `iced_runtime v0.14.0` | `MIT` |
| `iced_tiny_skia v0.14.1` | `MIT` |
| `iced_wayland_subscriber v0.21.0-dev (https://github.com/waycrate/exwlshelleventloop.git?branch=master#014ad7d6)` | `MIT` |
| `iced_wgpu v0.14.0` | `MIT` |
| `iced_widget v0.14.2` | `MIT` |
| `iced_winit v0.14.1` | `MIT` |
| `icu_collections v2.3.0` | `Unicode-3.0` |
| `icu_locale_core v2.3.0` | `Unicode-3.0` |
| `icu_normalizer v2.3.0` | `Unicode-3.0` |
| `icu_normalizer_data v2.3.0` | `Unicode-3.0` |
| `icu_properties v2.3.0` | `Unicode-3.0` |
| `icu_properties_data v2.3.0` | `Unicode-3.0` |
| `icu_provider v2.3.1` | `Unicode-3.0` |
| `ident_case v1.0.1` | `MIT/Apache-2.0` |
| `idna v1.1.0` | `MIT OR Apache-2.0` |
| `idna_adapter v1.2.2` | `Apache-2.0 OR MIT` |
| `image v0.25.10` | `MIT OR Apache-2.0` |
| `image-webp v0.2.4` | `MIT OR Apache-2.0` |
| `imagesize v0.13.0` | `MIT` |
| `imgref v1.12.3` | `CC0-1.0 OR Apache-2.0` |
| `indexmap v2.14.2` | `Apache-2.0 OR MIT` |
| `ini_core v0.2.0` | `MIT` |
| `itertools v0.14.0` | `MIT OR Apache-2.0` |
| `itoa v1.0.18` | `MIT OR Apache-2.0` |
| `kamadak-exif v0.6.1` | `BSD-2-Clause` |
| `keyboard-types v0.8.3` | `MIT OR Apache-2.0` |
| `khronos-egl v6.0.0` | `MIT/Apache-2.0` |
| `kurbo v0.10.4` | `MIT OR Apache-2.0` |
| `kurbo v0.11.3` | `Apache-2.0 OR MIT` |
| `lebe v0.5.3` | `BSD-3-Clause` |
| `libc v0.2.189` | `MIT OR Apache-2.0` |
| `libloading v0.8.9` | `ISC` |
| `libm v0.2.16` | `MIT` |
| `lilt v0.8.2` | `MIT` |
| `linebender_resource_handle v0.1.1` | `Apache-2.0 OR MIT` |
| `linux-raw-sys v0.12.1` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| `linux-raw-sys v0.4.15` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| `litemap v0.8.3` | `Unicode-3.0` |
| `litrs v1.0.0` | `MIT OR Apache-2.0` |
| `lock_api v0.4.14` | `MIT OR Apache-2.0` |
| `log v0.4.34` | `MIT OR Apache-2.0` |
| `loop9 v0.1.5` | `MIT` |
| `lru v0.16.4` | `MIT` |
| `lyon v1.0.19` | `MIT OR Apache-2.0` |
| `lyon_algorithms v1.0.21` | `MIT OR Apache-2.0` |
| `lyon_geom v1.0.19` | `MIT OR Apache-2.0` |
| `lyon_path v1.0.19` | `MIT OR Apache-2.0` |
| `lyon_tessellation v1.0.22` | `MIT OR Apache-2.0` |
| `manyhow v0.14.0` | `MIT OR Apache-2.0` |
| `manyhow-macros v0.14.0 (proc-macro)` | `MIT OR Apache-2.0` |
| `maybe-rayon v0.1.1` | `MIT` |
| `memchr v2.8.3` | `Unlicense OR MIT` |
| `memmap2 v0.9.11` | `MIT OR Apache-2.0` |
| `miniz_oxide v0.8.9` | `MIT OR Zlib OR Apache-2.0` |
| `miniz_oxide v0.9.1` | `MIT OR Zlib OR Apache-2.0` |
| `moxcms v0.8.1` | `BSD-3-Clause OR Apache-2.0` |
| `mundy v0.2.3` | `Apache-2.0` |
| `mutate_once v0.1.2` | `BSD-2-Clause` |
| `naga v27.0.3` | `MIT OR Apache-2.0` |
| `new_debug_unreachable v1.0.6` | `MIT` |
| `no_std_io2 v0.9.4` | `Apache-2.0 OR MIT` |
| `nom v8.0.0` | `MIT` |
| `noop_proc_macro v0.3.0 (proc-macro)` | `MIT` |
| `num-bigint v0.4.8` | `MIT OR Apache-2.0` |
| `num-complex v0.4.6` | `MIT OR Apache-2.0` |
| `num-derive v0.4.2 (proc-macro)` | `MIT OR Apache-2.0` |
| `num-integer v0.1.47` | `MIT OR Apache-2.0` |
| `num-rational v0.4.2` | `MIT OR Apache-2.0` |
| `num-traits v0.2.19` | `MIT OR Apache-2.0` |
| `once_cell v1.21.4` | `MIT OR Apache-2.0` |
| `option-ext v0.2.0` | `MPL-2.0` |
| `ordered-float v5.5.0` | `MIT` |
| `ordered-stream v0.2.0` | `MIT OR Apache-2.0` |
| `owned_ttf_parser v0.25.1` | `Apache-2.0` |
| `parking v2.2.1` | `Apache-2.0 OR MIT` |
| `parking_lot v0.12.5` | `MIT OR Apache-2.0` |
| `parking_lot_core v0.9.12` | `MIT OR Apache-2.0` |
| `paste v1.0.15 (proc-macro)` | `MIT OR Apache-2.0` |
| `pastey v0.1.1 (proc-macro)` | `MIT OR Apache-2.0` |
| `percent-encoding v2.3.2` | `MIT OR Apache-2.0` |
| `pico-args v0.5.0` | `MIT` |
| `pin-project-lite v0.2.17` | `Apache-2.0 OR MIT` |
| `piper v0.2.5` | `MIT OR Apache-2.0` |
| `pkg-config v0.3.34` | `MIT OR Apache-2.0` |
| `png v0.17.16` | `MIT OR Apache-2.0` |
| `png v0.18.1` | `MIT OR Apache-2.0` |
| `polling v3.11.0` | `Apache-2.0 OR MIT` |
| `potential_utf v0.1.6` | `Unicode-3.0` |
| `proc-macro-crate v3.5.0` | `MIT OR Apache-2.0` |
| `proc-macro-utils v0.10.0` | `MIT OR Apache-2.0` |
| `proc-macro2 v1.0.107` | `MIT OR Apache-2.0` |
| `profiling v1.0.18` | `MIT OR Apache-2.0` |
| `profiling-procmacros v1.0.18 (proc-macro)` | `MIT OR Apache-2.0` |
| `pulp v0.22.3` | `MIT` |
| `pulp-wasm-simd-flag v0.1.1` | `MIT` |
| `pxfm v0.1.30` | `BSD-3-Clause OR Apache-2.0` |
| `qoi v0.4.1` | `MIT/Apache-2.0` |
| `quick-error v2.0.1` | `MIT/Apache-2.0` |
| `quick-xml v0.41.0` | `MIT` |
| `quote v1.0.47` | `MIT OR Apache-2.0` |
| `rangemap v1.8.0` | `MIT/Apache-2.0` |
| `rav1e v0.8.1` | `BSD-2-Clause` |
| `ravif v0.13.0` | `BSD-3-Clause` |
| `raw-cpuid v11.6.0` | `MIT` |
| `raw-window-handle v0.6.2` | `MIT OR Apache-2.0 OR Zlib` |
| `rayon v1.12.0` | `MIT OR Apache-2.0` |
| `rayon-core v1.13.0` | `MIT OR Apache-2.0` |
| `read-fonts v0.35.0` | `MIT OR Apache-2.0` |
| `read-fonts v0.41.0` | `MIT OR Apache-2.0` |
| `reborrow v0.5.5` | `MIT` |
| `renderdoc-sys v1.1.0` | `MIT OR Apache-2.0` |
| `resvg v0.45.1` | `Apache-2.0 OR MIT` |
| `rgb v0.8.53` | `MIT` |
| `roxmltree v0.20.0` | `MIT OR Apache-2.0` |
| `rustc-hash v1.1.0` | `Apache-2.0/MIT` |
| `rustc-hash v2.1.3` | `Apache-2.0 OR MIT` |
| `rustix v0.38.44` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| `rustix v1.1.5` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| `rustybuzz v0.20.1` | `MIT` |
| `scoped-tls v1.0.1` | `MIT/Apache-2.0` |
| `scopeguard v1.2.0` | `MIT OR Apache-2.0` |
| `sctk-adwaita v0.10.1` | `MIT` |
| `self_cell v1.3.0` | `Apache-2.0 OR GPL-2.0-only` |
| `serde v1.0.229` | `MIT OR Apache-2.0` |
| `serde_core v1.0.229` | `MIT OR Apache-2.0` |
| `serde_derive v1.0.229 (proc-macro)` | `MIT OR Apache-2.0` |
| `serde_json v1.0.151` | `MIT OR Apache-2.0` |
| `serde_repr v0.1.21 (proc-macro)` | `MIT OR Apache-2.0` |
| `shlex v2.0.1` | `MIT OR Apache-2.0` |
| `signal-hook v0.3.18` | `Apache-2.0/MIT` |
| `signal-hook-registry v1.4.8` | `MIT OR Apache-2.0` |
| `simd-adler32 v0.3.10` | `MIT` |
| `simd_helpers v0.1.0 (proc-macro)` | `MIT` |
| `simplecss v0.2.2` | `Apache-2.0 OR MIT` |
| `siphasher v1.0.4` | `MIT OR Apache-2.0` |
| `skrifa v0.37.0` | `MIT OR Apache-2.0` |
| `skrifa v0.44.0` | `MIT OR Apache-2.0` |
| `slab v0.4.12` | `MIT` |
| `slotmap v1.1.1` | `Zlib` |
| `smallvec v1.16.2` | `MIT OR Apache-2.0` |
| `smithay-client-toolkit v0.19.2` | `MIT` |
| `smithay-client-toolkit v0.20.0` | `MIT` |
| `smithay-client-toolkit v0.21.1` | `MIT` |
| `smithay-clipboard v0.7.3` | `MIT` |
| `smol v2.0.2` | `Apache-2.0 OR MIT` |
| `smol_str v0.2.2` | `MIT OR Apache-2.0` |
| `smol_str v0.3.6` | `MIT OR Apache-2.0` |
| `softbuffer v0.4.8` | `MIT OR Apache-2.0` |
| `spirv v0.3.0+sdk-1.3.268.0` | `Apache-2.0` |
| `stable_deref_trait v1.2.1` | `MIT OR Apache-2.0` |
| `static_assertions v1.1.0` | `MIT OR Apache-2.0` |
| `strict-num v0.1.1` | `MIT` |
| `strsim v0.11.1` | `MIT` |
| `svg_fmt v0.4.5` | `MIT/Apache-2.0` |
| `svgtypes v0.15.3` | `Apache-2.0 OR MIT` |
| `swash v0.2.10` | `Apache-2.0 OR MIT` |
| `syn v2.0.119` | `MIT OR Apache-2.0` |
| `syn v3.0.6` | `MIT OR Apache-2.0` |
| `synstructure v0.14.0` | `MIT` |
| `sys-locale v0.3.2` | `MIT OR Apache-2.0` |
| `tempfile v3.27.0` | `MIT OR Apache-2.0` |
| `thiserror v1.0.69` | `MIT OR Apache-2.0` |
| `thiserror v2.0.21` | `MIT OR Apache-2.0` |
| `thiserror-impl v1.0.69 (proc-macro)` | `MIT OR Apache-2.0` |
| `thiserror-impl v2.0.21 (proc-macro)` | `MIT OR Apache-2.0` |
| `tiff v0.11.3` | `MIT` |
| `tiny-skia v0.11.4` | `BSD-3-Clause` |
| `tiny-skia-path v0.11.4` | `BSD-3-Clause` |
| `tiny-xlib v0.2.5` | `MIT OR Apache-2.0 OR Zlib` |
| `tinystr v0.8.4` | `Unicode-3.0` |
| `tinyvec v1.13.3` | `Zlib OR Apache-2.0 OR MIT` |
| `toml_datetime v1.1.1+spec-1.1.0` | `MIT OR Apache-2.0` |
| `toml_edit v0.25.15+spec-1.1.0` | `MIT OR Apache-2.0` |
| `toml_parser v1.1.3+spec-1.1.0` | `MIT OR Apache-2.0` |
| `tracing v0.1.44` | `MIT` |
| `tracing-attributes v0.1.31 (proc-macro)` | `MIT` |
| `tracing-core v0.1.36` | `MIT` |
| `ttf-parser v0.25.1` | `MIT OR Apache-2.0` |
| `unicode-bidi v0.3.18` | `MIT OR Apache-2.0` |
| `unicode-bidi-mirroring v0.4.0` | `MIT/Apache-2.0` |
| `unicode-ccc v0.4.0` | `MIT/Apache-2.0` |
| `unicode-ident v1.0.26` | `(MIT OR Apache-2.0) AND Unicode-3.0` |
| `unicode-linebreak v0.1.5` | `Apache-2.0` |
| `unicode-properties v0.1.4` | `MIT/Apache-2.0` |
| `unicode-script v0.5.8` | `MIT OR Apache-2.0` |
| `unicode-segmentation v1.13.3` | `MIT OR Apache-2.0` |
| `unicode-vo v0.1.0` | `MIT/Apache-2.0` |
| `unicode-width v0.2.2` | `MIT OR Apache-2.0` |
| `url v2.5.8` | `MIT OR Apache-2.0` |
| `usvg v0.45.1` | `Apache-2.0 OR MIT` |
| `utf8_iter v1.0.4` | `Apache-2.0 OR MIT` |
| `uuid v1.26.1` | `Apache-2.0 OR MIT` |
| `v_frame v0.3.9` | `BSD-2-Clause` |
| `version_check v0.9.5` | `MIT/Apache-2.0` |
| `waycrate_xkbkeycode v0.21.0-dev (https://github.com/waycrate/exwlshelleventloop.git?branch=master#014ad7d6)` | `MIT` |
| `wayland-backend v0.3.17` | `MIT` |
| `wayland-client v0.31.15` | `MIT` |
| `wayland-csd-frame v0.3.0` | `MIT` |
| `wayland-cursor v0.31.14` | `MIT` |
| `wayland-protocols v0.32.13` | `MIT` |
| `wayland-protocols-experimental v20250721.0.1` | `MIT` |
| `wayland-protocols-experimental v20251230.0.3` | `MIT` |
| `wayland-protocols-misc v0.3.12` | `MIT` |
| `wayland-protocols-plasma v0.3.12` | `MIT` |
| `wayland-protocols-wlr v0.3.12` | `MIT` |
| `wayland-scanner v0.31.11 (proc-macro)` | `MIT` |
| `wayland-sys v0.31.11` | `MIT` |
| `web-time v1.1.0` | `MIT OR Apache-2.0` |
| `weezl v0.1.12` | `MIT OR Apache-2.0` |
| `wgpu v27.0.1` | `MIT OR Apache-2.0` |
| `wgpu-core v27.0.3` | `MIT OR Apache-2.0` |
| `wgpu-core-deps-windows-linux-android v27.0.0` | `MIT OR Apache-2.0` |
| `wgpu-hal v27.0.4` | `MIT OR Apache-2.0` |
| `wgpu-types v27.0.1` | `MIT OR Apache-2.0` |
| `window_clipboard v0.5.1` | `MIT` |
| `winit v0.30.13` | `Apache-2.0` |
| `winit-common v0.31.0-beta.3` | `Apache-2.0` |
| `winit-core v0.31.0-beta.3` | `Apache-2.0` |
| `winnow v1.0.4` | `MIT` |
| `writeable v0.6.4` | `Unicode-3.0` |
| `x11-dl v2.21.0` | `MIT` |
| `x11rb v0.13.2` | `MIT OR Apache-2.0` |
| `x11rb-protocol v0.13.2` | `MIT OR Apache-2.0` |
| `xcursor v0.3.11` | `MIT` |
| `xdg v2.5.2` | `Apache-2.0 OR MIT` |
| `xkbcommon v0.8.0` | `MIT` |
| `xkbcommon-dl v0.4.2` | `MIT` |
| `xkeysym v0.2.1` | `MIT OR Apache-2.0 OR Zlib` |
| `xmlwriter v0.1.0` | `MIT` |
| `y4m v0.8.0` | `MIT` |
| `yazi v0.2.1` | `Apache-2.0 OR MIT` |
| `yoke v0.8.3` | `Unicode-3.0` |
| `yoke-derive v0.8.4 (proc-macro)` | `Unicode-3.0` |
| `zbus v5.19.0` | `MIT` |
| `zbus_macros v5.19.0 (proc-macro)` | `MIT` |
| `zbus_names v4.3.4` | `MIT` |
| `zcheapstr v1.1.0` | `MIT` |
| `zeno v0.3.3` | `Apache-2.0 OR MIT` |
| `zerocopy v0.8.59` | `BSD-2-Clause OR Apache-2.0 OR MIT` |
| `zerocopy-derive v0.8.59 (proc-macro)` | `BSD-2-Clause OR Apache-2.0 OR MIT` |
| `zerofrom v0.1.8` | `Unicode-3.0` |
| `zerofrom-derive v0.1.8 (proc-macro)` | `Unicode-3.0` |
| `zerotrie v0.2.5` | `Unicode-3.0` |
| `zerovec v0.11.8` | `Unicode-3.0` |
| `zerovec-derive v0.11.6 (proc-macro)` | `Unicode-3.0` |
| `zmij v1.0.23` | `MIT` |
| `zune-core v0.4.12` | `MIT OR Apache-2.0 OR Zlib` |
| `zune-core v0.5.3` | `MIT OR Apache-2.0 OR Zlib` |
| `zune-inflate v0.2.54` | `MIT OR Apache-2.0 OR Zlib` |
| `zune-jpeg v0.4.21` | `MIT OR Apache-2.0 OR Zlib` |
| `zune-jpeg v0.5.15` | `MIT OR Apache-2.0 OR Zlib` |
| `zvariant v5.15.0` | `MIT` |
| `zvariant_derive v5.15.0 (proc-macro)` | `MIT` |
| `zvariant_utils v4.2.0` | `MIT` |
