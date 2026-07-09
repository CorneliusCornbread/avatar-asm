//! pack: builds a minimal .glb containing a WASM module referenced by a
//! (prototype, non-final) `EXT_animator_wasm` node extension.
//!
//! Usage: cargo run --bin pack -- out.glb
//!
//! This intentionally does NOT implement the full WASM Host API contract
//! from anim_prototype.md (no jointBuffer/morphWeightBuffer/sampleClip
//! imports). It's the smallest possible thing that proves the pipeline:
//! WASM bytes -> bufferView -> node extension -> .glb -> load -> execute.

use anyhow::{Context, Result};
use gltf::binary::{Glb, Header};
use std::borrow::Cow;
use std::env;

/// A minimal, WASI-free "hello world" module.
///
/// - Exports `memory` so the host can read linear memory directly (same
///   pattern the real host API uses for jointBuffer/morphWeightBuffer).
/// - Exports `greet` -> i32, a pointer to a nul-terminated UTF-8 string
///   baked into a data segment.
/// - Exports stub `init`/`evaluate` matching the real WASM Module Contract's
///   export shape, so the runner can exercise that call sequence too, even
///   though this module ignores the arguments.
///
/// No imports at all -- this module doesn't touch the "env" host API yet.
const HELLO_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 8) "Hello, Nexus! This string was read out of a WASM module's linear memory by the host.\00")

  (func (export "greet") (result i32)
    i32.const 8)

  (func (export "init") (param $jointCount i32) (param $morphCount i32))
  (func (export "evaluate") (param $deltaTime f32))
)
"#;

fn main() -> Result<()> {
    let out_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "hello_avatar.glb".to_string());

    // 1. Compile the WAT source down to real WASM bytecode. This is exactly
    //    the bytes a compiler toolchain (e.g. wasm-pack, or the eventual
    //    Unity -> WASM compiler) would hand you.
    let wasm_bytes: Vec<u8> = wat::parse_str(HELLO_WAT).context("compiling WAT to WASM")?;
    println!("compiled WASM module: {} bytes", wasm_bytes.len());

    // 2. The GLB binary payload (the "BIN" chunk) is just the raw WASM
    //    bytes. glTF buffers can hold arbitrary binary data -- there's
    //    nothing WASM-specific about a buffer/bufferView, we're just
    //    parking bytes there and pointing at them from an extension.
    let bin_chunk = wasm_bytes;

    // 3. Build the glTF JSON by hand. EXT_animator_wasm isn't a standard
    //    extension gltf-rs knows about, so rather than fight the typed
    //    gltf_json API we just construct the JSON document directly --
    //    which is a perfectly valid way to author glTF, since the format
    //    IS the JSON.
    let doc = serde_json::json!({
        "asset": { "version": "2.0", "generator": "nexus-anim pack MVP" },
        "extensionsUsed": ["EXT_animator_wasm"],
        // Note: EXT_animator_wasm is NOT in extensionsRequired here on
        // purpose -- see the writeup for why that's a real open question
        // for the spec, not just an MVP shortcut.
        "scene": 0,
        "scenes": [ { "nodes": [0] } ],
        "nodes": [
            {
                "name": "AvatarRoot",
                // A real avatar node would also carry "skin": <index>.
                // Omitted here since this MVP has no skeleton yet.
                "extensions": {
                    "EXT_animator_wasm": {
                        "bufferView": 0,
                        "hostApiVersion": "0.0-hello-world",
                        "applyRootMotion": false,
                        "updateMode": "normal"
                    }
                }
            }
        ],
        "buffers": [
            { "byteLength": bin_chunk.len() }
            // No "uri" -- this buffer is implicitly the GLB's BIN chunk.
        ],
        "bufferViews": [
            {
                "buffer": 0,
                "byteOffset": 0,
                "byteLength": bin_chunk.len()
                // No "target" -- this isn't vertex/index data, it's opaque
                // bytes for our extension to interpret. Real WASM bytes
                // should probably get an explicit mimeType of
                // "application/wasm" once EXT_animator_wasm is formalized;
                // core glTF bufferViews don't have a mimeType field today
                // (only image sources do), so that'll need to be part of
                // the extension spec itself, not core glTF.
            }
        ]
    });

    let json_bytes = serde_json::to_vec(&doc).context("serializing glTF JSON")?;

    // 4. Pack JSON chunk + BIN chunk into a single .glb container.
    //    gltf::binary::Glb::to_writer handles the 12-byte header, chunk
    //    headers, magic bytes, and 4-byte alignment padding for us.
    let glb = Glb {
        header: Header {
            magic: *b"glTF",
            version: 2,
            length: 0, // recomputed by to_writer; this value is unused on write
        },
        json: Cow::Owned(json_bytes),
        bin: Some(Cow::Owned(bin_chunk)),
    };

    let file = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {out_path}"))?;
    glb.to_writer(file).context("writing .glb")?;

    println!("wrote {out_path}");
    Ok(())
}
