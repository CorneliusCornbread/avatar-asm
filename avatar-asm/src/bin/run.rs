//! run: loads a .glb produced by `pack`, extracts the WASM module referenced
//! by its EXT_animator_wasm node extension, and executes it with wasmtime.
//!
//! Usage: cargo run --bin run -- hello_avatar.glb

use wasmtime::error::Context as wasm;
use anyhow::{anyhow, Result};
use gltf::binary::Glb;
use std::env;
use wasmtime::{Engine, Instance, Module, Store, TypedFunc};

fn main() -> Result<()> {
    let in_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "hello_avatar.glb".to_string());

    let file_bytes = anyhow::Context::with_context(std::fs::read(&in_path), || format!("reading {in_path}"))?;

    // 1. Split the .glb into its JSON chunk and BIN chunk. This is pure
    //    container parsing -- no glTF semantics yet.
    let glb = anyhow::Context::context(Glb::from_slice(&file_bytes), "parsing .glb container")?;
    let bin = glb
        .bin
        .as_ref()
        .ok_or_else(|| anyhow!("glb has no BIN chunk"))?;

    // 2. Parse the JSON chunk generically. We deliberately use
    //    serde_json::Value instead of gltf::json::Root here: Root's
    //    `extensions` fields are typed to extensions gltf-rs knows about,
    //    and EXT_animator_wasm isn't one of them (yet). Reading it as raw
    //    JSON keeps this decoupled from gltf-rs's extension allowlist.
    let doc: serde_json::Value =
        anyhow::Context::context(serde_json::from_slice(&glb.json), "parsing glTF JSON chunk")?;

    // 3. Find the node carrying EXT_animator_wasm and read its bufferView
    //    index. In a real loader you'd walk scenes[]->nodes[] properly and
    //    handle multiple avatars; for the MVP we just grab node 0.
    let ext = &doc["nodes"][0]["extensions"]["EXT_animator_wasm"];
    let buffer_view_index = ext["bufferView"]
        .as_u64()
        .ok_or_else(|| anyhow!("node 0 has no EXT_animator_wasm.bufferView"))?
        as usize;
    let host_api_version = ext["hostApiVersion"].as_str().unwrap_or("<missing>");
    println!("found EXT_animator_wasm, hostApiVersion = {host_api_version}");

    // 4. Resolve the bufferView to a byte range and slice it out of the
    //    BIN chunk. (Not handling buffer != 0 or a "uri" buffer here --
    //    an MVP loader only needs to support the GLB-embedded case.)
    let buffer_view = &doc["bufferViews"][buffer_view_index];
    let byte_offset = buffer_view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let byte_length = buffer_view["byteLength"]
        .as_u64()
        .ok_or_else(|| anyhow!("bufferView {buffer_view_index} has no byteLength"))?
        as usize;
    let wasm_bytes = &bin[byte_offset..byte_offset + byte_length];
    println!("extracted {} bytes of WASM from bufferView {buffer_view_index}", wasm_bytes.len());

    // 5. Compile and instantiate with wasmtime. Per the project's own perf
    //    recommendations, real clients must compile untrusted .wasm bytes
    //    from scratch (Module::from_binary/new does full validation) rather
    //    than deserializing a pre-AOT'd artifact from the network -- see
    //    nexus_anim_wasm_perf_recommendations.md. This module has no
    //    "env" imports, so the linker below is empty; the full host API
    //    (getJointBufferPtr, sampleClip, getParamFloat, ...) will need a
    //    populated Linker once EXT_animator_wasm modules start using it.
    let engine = Engine::default();
    let module = Module::new(&engine, wasm_bytes).context("compiling WASM module")?;
    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[]).context("instantiating WASM module")?;

    // 6. Exercise the WASM Module Contract's export shape: init(...) then
    //    evaluate(...), matching the real per-frame call sequence, even
    //    though this module's implementations are no-ops.
    let init: TypedFunc<(i32, i32), ()> = instance
        .get_typed_func(&mut store, "init")
        .context("module missing init export")?;
    init.call(&mut store, (0, 0)).context("calling init")?;

    let evaluate: TypedFunc<f32, ()> = instance
        .get_typed_func(&mut store, "evaluate")
        .context("module missing evaluate export")?;
    evaluate.call(&mut store, 1.0 / 60.0).context("calling evaluate")?;

    // 7. Call greet() -> pointer, then read a nul-terminated string directly
    //    out of the module's exported linear memory. This is the same
    //    read-shared-memory pattern the real jointBuffer/morphWeightBuffer
    //    host API will use, just with a string instead of pose floats.
    let greet: TypedFunc<(), i32> = instance
        .get_typed_func(&mut store, "greet")
        .context("module missing greet export")?;
    let ptr = greet.call(&mut store, ())? as usize;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("module has no exported memory"))?;
    let data = memory.data(&store);
    let end = data[ptr..]
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| anyhow!("greeting string not nul-terminated"))?;
    let greeting = std::str::from_utf8(&data[ptr..ptr + end])?;

    println!("\nWASM says: \"{greeting}\"");

    Ok(())
}
