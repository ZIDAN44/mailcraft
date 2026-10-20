//! The Windows application manifest embedded in mailcraft.exe (`build.rs`): per-monitor DPI
//! awareness (V2, with per-monitor V1 for older Windows 10), declared before any code runs, as
//! PdfCraft does. winit sets it at run time too, but only once the window is being made; anything
//! shown before, such as the message when the window can't open, was drawn blurred and scaled.

pub(crate) const WINDOWS_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0" xmlns:asmv3="urn:schemas-microsoft-com:asm.v3">
  <asmv3:application>
    <asmv3:windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2, PerMonitor</dpiAwareness>
    </asmv3:windowsSettings>
  </asmv3:application>
</assembly>
"#;

#[cfg(test)]
mod tests {
    use super::WINDOWS_MANIFEST;

    #[test]
    fn declares_per_monitor_dpi_awareness_and_is_embedded() {
        assert!(WINDOWS_MANIFEST.contains(">PerMonitorV2, PerMonitor</dpiAwareness>"));
        assert!(WINDOWS_MANIFEST.contains(">true/pm</dpiAware>"));
        assert!(include_str!("../build.rs").contains("set_manifest(windows_manifest::WINDOWS_MANIFEST)"));
    }
}
