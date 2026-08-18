//! `build`：把校验过的 manifest + widget.ftui + plugin.wasm 打包为 `.floatile`。
//!
//! 打包后立即用 `validate_package` 自校验，确保产物是可安装的。

use std::io::Write;
use std::path::Path;

use floatile_core::manifest::Manifest;

use crate::package::{PackageError, validate_package};

/// 打包 `.floatile` zip 并自校验。`out` 为输出文件路径（`.floatile`）。
pub fn package(
    manifest: &Manifest,
    ftui_json: &str,
    wasm: &[u8],
    out: &Path,
) -> Result<(), PackageError> {
    let manifest_json =
        serde_json::to_string(manifest).map_err(|e| PackageError::InvalidJson(e.to_string()))?;

    let mut bytes = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default();
        writer
            .start_file("manifest.json", options)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .write_all(manifest_json.as_bytes())
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .start_file("ui/widget.ftui", options)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .write_all(ftui_json.as_bytes())
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .start_file("logic/plugin.wasm", options)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .write_all(wasm)
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    }

    // 自校验：产物必须通过包校验。
    validate_package(&bytes, &crate::package::PackageLimits::default())?;

    let mut file =
        std::fs::File::create(out).map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    file.write_all(&bytes)
        .map_err(|e| PackageError::CorruptZip(e.to_string()))?;
    Ok(())
}
