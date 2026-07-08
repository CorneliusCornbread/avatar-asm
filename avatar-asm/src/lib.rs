use gltf::Gltf;
use wasmtime::Engine;

pub struct GltfAvatar {
    gltf: Gltf,
    wasm_engine: Engine,
}
