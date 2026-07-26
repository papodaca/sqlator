use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let data_dir = manifest_dir.join("data");
    let blp_dir = data_dir.join("ui");
    let ui_out = out_dir.join("ui");
    let schemas_out = out_dir.join("schemas");

    fs::create_dir_all(&ui_out).expect("create OUT_DIR/ui");
    fs::create_dir_all(&schemas_out).expect("create OUT_DIR/schemas");

    // 1. Blueprint → .ui (fail clearly if compiler missing)
    let blp_files = collect_files(&blp_dir, "blp");
    for blp in &blp_files {
        println!("cargo:rerun-if-changed={}", blp.display());
    }
    if !blp_files.is_empty() {
        let status = Command::new("blueprint-compiler")
            .arg("batch-compile")
            .arg(&ui_out)
            .arg(&blp_dir)
            .args(&blp_files)
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => panic!("blueprint-compiler failed with status {s}"),
            Err(e) => panic!(
                "blueprint-compiler is required to build sqlator-gtk (not found: {e}). \
                 Install blueprint-compiler >= 0.22."
            ),
        }
    }

    // Hand-written .ui (if any) also invalidate the build.
    for ui in collect_files(&data_dir.join("ui"), "ui") {
        println!("cargo:rerun-if-changed={}", ui.display());
    }

    // CSS / icons / gresource xml
    let gresource_xml = data_dir.join("sqlator.gresource.xml");
    let style_css = data_dir.join("style.css");
    println!("cargo:rerun-if-changed={}", gresource_xml.display());
    println!("cargo:rerun-if-changed={}", style_css.display());

    // 2. Compile GResource — OUT_DIR first so generated OUT_DIR/ui/*.ui wins;
    // data/ supplies style.css / icons / any hand-written .ui under data/ui/.
    // gresource paths are relative to these sourcedirs (e.g. "ui/window.ui").
    glib_build_tools::compile_resources(
        &[&out_dir, &data_dir],
        gresource_xml.to_str().unwrap(),
        "sqlator.gresource",
    );

    // 3. GSettings schema into OUT_DIR/schemas for cargo-run without install
    let schema_src = data_dir.join("im.apodaca.SqlatorGtk.gschema.xml");
    println!("cargo:rerun-if-changed={}", schema_src.display());
    fs::copy(
        &schema_src,
        schemas_out.join("im.apodaca.SqlatorGtk.gschema.xml"),
    )
    .expect("copy gschema into OUT_DIR/schemas");
    let schema_status = Command::new("glib-compile-schemas")
        .arg(&schemas_out)
        .status()
        .expect("glib-compile-schemas must be available");
    if !schema_status.success() {
        panic!("glib-compile-schemas failed with status {schema_status}");
    }
    println!(
        "cargo:rustc-env=SQLATOR_SCHEMA_DIR={}",
        schemas_out.display()
    );
}

fn collect_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            files.push(path);
        }
    }
    files.sort();
    files
}
