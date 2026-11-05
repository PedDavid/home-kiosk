// The frontend is embedded from frontend/dist (built with `npm run build` in frontend/). Make sure
// the folder exists so a plain `cargo build`/`cargo test` works without Node; the server then
// answers pages with a hint to build the frontend.
fn main() {
    let dist = std::path::Path::new("frontend/dist");
    if !dist.exists() {
        std::fs::create_dir_all(dist).expect("create frontend/dist");
    }
    println!("cargo:rerun-if-changed=frontend/dist");
}
