//! The graphics adapter MailCraft's window is drawn with.
//!
//! On Windows only DirectX 12 starts, and the adapter is picked here: a real GPU first, WARP
//! (Windows' own software renderer, which DirectX 12 lists as "Microsoft Basic Render Driver")
//! when there is none. wgpu otherwise starts Vulkan and OpenGL too. On a PC without a GPU those
//! find only leftover or overlay drivers and Windows' OpenGL 1.1 software renderer, and the
//! window never opened: the process stayed in the background holding the data files, so later
//! launches were told another copy had them open. DirectX 12 works there through WARP.
//!
//! `WGPU_BACKEND` still chooses the backends (and then wgpu picks the adapter, as before),
//! `WGPU_POWER_PREF=low` puts an integrated GPU before a discrete one, and
//! `WGPU_FORCE_FALLBACK_ADAPTER=1` draws with WARP, as a PC without a GPU does. The shaders are
//! compiled with FXC, part of Windows (`WGPU_DX12_COMPILER` chooses another): wgpu's default loads
//! whichever `dxcompiler.dll` the DLL search path finds, and one without its `dxil.dll` loses the
//! device before the first frame.
//!
//! Everywhere, what the Craft apps learnt on users' machines:
//! - the device asks for what the adapter has ([`device_limits`]): egui-wgpu asks for fixed limits,
//!   which some GPUs fall short of, and for 8192-pixel textures at most, so a window wider than that
//!   (3440 points at 250%) panicked configuring its surface;
//! - on Intel Macs wgpu's indirect-call validation shaders are left out: Metal's Intel compiler
//!   aborts on them and the window never opens (MailCraft draws nothing indirect);
//! - on Linux and the BSDs the window is drawn on the GPU the desktop runs on, not always the
//!   discrete one, which on a PRIME laptop showed stale or garbled frames;
//! - once the window has drawn its first frames, a wgpu error is logged rather than ending the app
//!   ([`log_errors`]); before that it still ends the start, which then says why.

use eframe::wgpu::{self, DeviceType, PowerPreference};

/// The order adapters are tried in (lowest first): a real GPU (the discrete one, or with
/// `integrated_first` the integrated one), then one of unknown or virtual kind, then software.
pub fn rank(device: DeviceType, integrated_first: bool) -> u8 {
    match device {
        DeviceType::DiscreteGpu => 1 + u8::from(integrated_first),
        DeviceType::IntegratedGpu => 2 - u8::from(integrated_first),
        DeviceType::Other => 3,
        DeviceType::VirtualGpu => 4,
        DeviceType::Cpu => 5,
    }
}

/// One adapter as the choice sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub device: DeviceType,
    /// It can present to the window.
    pub draws: bool,
}

/// The adapter to draw with, by its place in `found`: the best-ranked one that can draw to the
/// window (with `software_only`, WARP alone). `Err` says what was found, for the message shown
/// when the window can't open.
pub fn choose(found: &[Candidate], integrated_first: bool, software_only: bool) -> Result<usize, String> {
    found
        .iter()
        .enumerate()
        .filter(|(_, c)| c.draws && (!software_only || c.device == DeviceType::Cpu))
        .min_by_key(|(at, c)| (rank(c.device, integrated_first), *at))
        .map(|(at, _)| at)
        .ok_or_else(|| {
            if found.is_empty() {
                "DirectX 12 found no graphics adapter, not even Windows' software renderer".to_string()
            } else {
                let names: Vec<String> =
                    found.iter().map(|c| format!("{} ({:?}{})", c.name, c.device, if c.draws { "" } else { ", can't draw to the window" })).collect();
                let what = if software_only { "Windows' software renderer (WARP)" } else { "a graphics adapter" };
                format!("{what} can't draw the window; DirectX 12 found {}", names.join(", "))
            }
        })
}

/// Whether `WGPU_FORCE_FALLBACK_ADAPTER` asks for the software adapter.
fn software_only() -> bool {
    std::env::var("WGPU_FORCE_FALLBACK_ADAPTER").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// Sets up the window's graphics as the module says.
pub fn configure(options: &mut eframe::NativeOptions) {
    if cfg!(windows) {
        windows_setup(options, wgpu::Backends::from_env(), wgpu::Dx12Compiler::from_env(), software_only());
    }
    common_setup(options, cfg!(target_os = "macos") && cfg!(target_arch = "x86_64"), cfg!(all(unix, not(target_os = "macos"))));
}

/// The limits to ask the device for: egui-wgpu's `required`, no more than the adapter has
/// (`supported`), and the adapter's whole texture size, so a window as large as the GPU can draw
/// can be drawn (egui-wgpu asks for 8192 pixels).
pub fn device_limits(required: wgpu::Limits, supported: &wgpu::Limits) -> wgpu::Limits {
    let mut limits = required.or_worse_values_from(supported);
    limits.max_texture_dimension_2d = supported.max_texture_dimension_2d;
    limits
}

/// Every platform: the device's limits from the adapter; with `intel_mac`, no indirect-call
/// validation (`WGPU_VALIDATION_INDIRECT_CALL=1` turns it back on); with `desktop_gpu`, no power
/// preference, so wgpu takes the GPU the desktop runs on (`WGPU_POWER_PREF` still chooses).
fn common_setup(options: &mut eframe::NativeOptions, intel_mac: bool, desktop_gpu: bool) {
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup else { return };
    let asked = std::sync::Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = std::sync::Arc::new(move |adapter: &wgpu::Adapter| {
        let mut d = asked(adapter);
        d.required_limits = device_limits(d.required_limits, &adapter.limits());
        d
    });
    if intel_mac {
        setup.instance_descriptor.flags = setup.instance_descriptor.flags.difference(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL).with_env();
    }
    if desktop_gpu {
        setup.power_preference = PowerPreference::from_env().unwrap_or(PowerPreference::None);
    }
}

/// How many frames the window draws before a wgpu error is only logged (see [`log_errors`]).
pub const STARTUP_FRAMES: u32 = 3;

/// From now on, a wgpu error on `device` (a validation error, running out of memory) is written to
/// mailcraft.log instead of ending the app: wgpu's own handler panics. Called once the window is
/// past its first frames, so a GPU that can't start still ends the start with its reason.
pub fn log_errors(device: &wgpu::Device) {
    device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| log::error!("graphics: {e}")));
}

/// Whether this is a 32-bit build (the Windows x86 release).
fn thirty_two_bit() -> bool {
    cfg!(target_pointer_width = "32")
}

/// DirectX 12 alone (unless `backends` overrides it; OpenGL for a 32-bit build), shaders compiled with `compiler` or FXC,
/// and the adapter chosen by [`choose`].
fn windows_setup(options: &mut eframe::NativeOptions, backends: Option<wgpu::Backends>, compiler: Option<wgpu::Dx12Compiler>, software: bool) {
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup else { return };
    setup.instance_descriptor.backend_options.dx12.shader_compiler = compiler.unwrap_or(wgpu::Dx12Compiler::Fxc);
    if let Some(b) = backends {
        log::info!("graphics: WGPU_BACKEND asks for {b:?}; wgpu picks the adapter");
        setup.instance_descriptor.backends = b;
        return;
    }
    // A 32-bit MailCraft: DirectX 12 can't make its pipelines in a 32-bit process ("The
    // parameter is incorrect", the window never opened), so OpenGL draws it, wgpu picking.
    if thirty_two_bit() {
        log::info!("graphics: a 32-bit process draws with OpenGL");
        setup.instance_descriptor.backends = wgpu::Backends::GL;
        return;
    }
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    let integrated_first = setup.power_preference == PowerPreference::LowPower;
    setup.native_adapter_selector = Some(std::sync::Arc::new(move |adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>| {
        let found: Vec<Candidate> = adapters
            .iter()
            .map(|a| {
                let info = a.get_info();
                Candidate { name: info.name, device: info.device_type, draws: surface.is_none_or(|s| a.is_surface_supported(s)) }
            })
            .collect();
        for c in &found {
            log::info!("graphics: DirectX 12 adapter {:?} ({:?}){}", c.name, c.device, if c.draws { "" } else { ", can't draw to the window" });
        }
        let at = choose(&found, integrated_first, software).inspect_err(|e| log::error!("graphics: {e}"))?;
        let chosen = adapters.get(at).cloned().ok_or_else(|| "the chosen graphics adapter went away".to_string())?;
        if let Some(c) = found.get(at) {
            let how = if c.device == DeviceType::Cpu { " (software: drawn by the processor)" } else { "" };
            log::info!("graphics: drawing with {:?}{how}", c.name);
        }
        Ok(chosen)
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(list: &[(&str, DeviceType)]) -> Vec<Candidate> {
        list.iter().map(|(n, d)| Candidate { name: (*n).to_string(), device: *d, draws: true }).collect()
    }

    /// A PC without a GPU: DirectX 12 offers WARP alone, and the window is drawn with it rather
    /// than not opening.
    #[test]
    fn without_a_gpu_the_software_renderer_draws() {
        let warp = found(&[("Microsoft Basic Render Driver", DeviceType::Cpu)]);
        assert_eq!(choose(&warp, false, false), Ok(0));
    }

    /// A real GPU comes before WARP, whatever the order DirectX lists them in; the discrete one
    /// first, or the integrated one when asked to save power.
    #[test]
    fn a_real_gpu_comes_first() {
        let three = found(&[
            ("Microsoft Basic Render Driver", DeviceType::Cpu),
            ("AMD Radeon(TM) Graphics", DeviceType::IntegratedGpu),
            ("NVIDIA GeForce RTX 4070 Ti SUPER", DeviceType::DiscreteGpu),
        ]);
        assert_eq!(choose(&three, false, false), Ok(2));
        assert_eq!(choose(&three, true, false), Ok(1));
        // Asked for software (WGPU_FORCE_FALLBACK_ADAPTER), WARP even next to a GPU.
        assert_eq!(choose(&three, false, true), Ok(0));
        // A GPU that can't draw to the window is passed over for one that can.
        let mut weak = three.clone();
        if let Some(n) = weak.get_mut(2) {
            n.draws = false;
        }
        assert_eq!(choose(&weak, false, false), Ok(1));
    }

    /// Nothing that can draw: an error naming what was found, for the message box.
    #[test]
    fn nothing_to_draw_with_says_what_was_found() {
        let e = choose(&[], false, false).unwrap_err();
        assert!(e.contains("no graphics adapter"), "{e}");
        let only_gpu = found(&[("Intel(R) UHD Graphics", DeviceType::IntegratedGpu)]);
        let e = choose(&only_gpu, false, true).unwrap_err();
        assert!(e.contains("software renderer") && e.contains("Intel(R) UHD Graphics"), "{e}");
    }

    /// The device asks for no more than the adapter has, and for its whole texture size (a 3440-point
    /// window at 250% needs 8600 pixels; egui-wgpu asked for 8192).
    #[test]
    fn the_device_asks_for_what_the_adapter_has() {
        let required = wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() };
        let big = wgpu::Limits { max_texture_dimension_2d: 16384, ..wgpu::Limits::default() };
        assert_eq!(device_limits(required.clone(), &big).max_texture_dimension_2d, 16384);
        // A GPU short of the defaults (Raspberry Pi's V3D: 15 inter-stage variables, 4096 pixels).
        let small = wgpu::Limits { max_texture_dimension_2d: 4096, max_inter_stage_shader_variables: 15, ..wgpu::Limits::default() };
        let got = device_limits(required, &small);
        assert!(got.check_limits(&small), "{got:?}");
        assert_eq!((got.max_texture_dimension_2d, got.max_inter_stage_shader_variables), (4096, 15));
    }

    /// Intel Macs leave the indirect-call validation out; other platforms keep wgpu's flags. Linux
    /// and the BSDs ask for no power preference unless told one.
    #[test]
    fn intel_macs_and_linux_get_their_own_setup() {
        let setup_of = |intel_mac: bool, desktop_gpu: bool| {
            let mut o = eframe::NativeOptions::default();
            common_setup(&mut o, intel_mac, desktop_gpu);
            match o.wgpu_options.wgpu_setup {
                eframe::egui_wgpu::WgpuSetup::CreateNew(s) => Some(s),
                _ => None,
            }
        };
        let plain = setup_of(false, false).unwrap();
        if std::env::var_os("WGPU_VALIDATION_INDIRECT_CALL").is_none() {
            let mac = setup_of(true, false).unwrap();
            assert!(!mac.instance_descriptor.flags.contains(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL));
        }
        assert_eq!(plain.power_preference, PowerPreference::from_env().unwrap_or(PowerPreference::HighPerformance));
        let linux = setup_of(false, true).unwrap();
        assert_eq!(linux.power_preference, PowerPreference::from_env().unwrap_or(PowerPreference::None));
    }

    /// DirectX 12 only, never Vulkan or OpenGL (eframe's default starts both), shaders with FXC,
    /// and the adapter picked here; `WGPU_BACKEND` and `WGPU_DX12_COMPILER` still win.
    #[test]
    fn windows_starts_directx_12_alone() {
        let setup_of = |o: eframe::NativeOptions| match o.wgpu_options.wgpu_setup {
            eframe::egui_wgpu::WgpuSetup::CreateNew(s) => Some(s),
            _ => None,
        };
        let mut o = eframe::NativeOptions::default();
        windows_setup(&mut o, None, None, false);
        let s = setup_of(o).unwrap();
        if thirty_two_bit() {
            assert_eq!(s.instance_descriptor.backends, wgpu::Backends::GL, "a 32-bit build never asks DirectX 12");
            return;
        }
        assert_eq!(s.instance_descriptor.backends, wgpu::Backends::DX12);
        assert!(matches!(s.instance_descriptor.backend_options.dx12.shader_compiler, wgpu::Dx12Compiler::Fxc));
        assert!(s.native_adapter_selector.is_some());

        let mut o = eframe::NativeOptions::default();
        windows_setup(&mut o, Some(wgpu::Backends::VULKAN), Some(wgpu::Dx12Compiler::Auto), false);
        let s = setup_of(o).unwrap();
        assert_eq!(s.instance_descriptor.backends, wgpu::Backends::VULKAN);
        assert!(matches!(s.instance_descriptor.backend_options.dx12.shader_compiler, wgpu::Dx12Compiler::Auto));
        assert!(s.native_adapter_selector.is_none(), "wgpu picks the adapter for backends asked for by name");
    }
}
