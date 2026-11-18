//! Set GPU limits from the adapter so older Android devices can start the window.

use eframe::wgpu;

pub(super) fn configure(options: &mut eframe::NativeOptions) {
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup else { return };
    let requested_backends = wgpu::Backends::from_env();
    let (backends, probe_vulkan) = requested_backends.map_or_else(backend_for_device, |backends| (backends, false));
    setup.instance_descriptor.backends = backends;
    if probe_vulkan {
        setup.native_adapter_selector = Some(std::sync::Arc::new(select_vulkan_adapter));
    }
    let requested = std::sync::Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = std::sync::Arc::new(move |adapter: &wgpu::Adapter| {
        let mut descriptor = requested(adapter);
        descriptor.required_limits = device_limits(descriptor.required_limits, &adapter.limits());
        descriptor
    });
}

fn backend_for_device() -> (wgpu::Backends, bool) {
    let probed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN)).iter().any(|adapter| usable_buffer_limits(&adapter.limits()))
    }));
    if matches!(probed, Ok(true)) {
        log::info!("Android graphics: using Vulkan");
        (wgpu::Backends::VULKAN, true)
    } else {
        log::info!("Android graphics: Vulkan has no usable buffer limits; using OpenGL ES");
        (wgpu::Backends::GL, false)
    }
}

fn usable_buffer_limits(limits: &wgpu::Limits) -> bool {
    limits.max_buffer_size > 0 && limits.max_uniform_buffer_binding_size > 0
}

fn select_vulkan_adapter(adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>) -> Result<wgpu::Adapter, String> {
    adapters
        .iter()
        .enumerate()
        .filter(|(_, adapter)| surface.is_none_or(|surface| adapter.is_surface_supported(surface)) && usable_buffer_limits(&adapter.limits()))
        .min_by_key(|(index, adapter)| {
            let device_rank = match adapter.get_info().device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::VirtualGpu => 2,
                wgpu::DeviceType::Other => 3,
                wgpu::DeviceType::Cpu => 4,
            };
            (device_rank, *index)
        })
        .map(|(_, adapter)| adapter.clone())
        .ok_or_else(|| "Android graphics found no Vulkan adapter with usable buffers".into())
}

fn device_limits(required: wgpu::Limits, supported: &wgpu::Limits) -> wgpu::Limits {
    let mut limits = required.or_worse_values_from(supported);
    limits.max_texture_dimension_2d = supported.max_texture_dimension_2d;
    limits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_limits_fit_the_android_adapter() {
        let required = wgpu::Limits::default();
        let supported = wgpu::Limits {
            max_buffer_size: 32 << 20,
            max_texture_dimension_2d: 4096,
            max_inter_stage_shader_variables: 15,
            ..wgpu::Limits::default()
        };
        let got = device_limits(required, &supported);
        assert!(got.check_limits(&supported), "{got:?}");
        assert_eq!((got.max_buffer_size, got.max_texture_dimension_2d, got.max_inter_stage_shader_variables), (32 << 20, 4096, 15));
    }

    #[test]
    fn android_vulkan_probe_rejects_zero_buffer_limits() {
        assert!(!usable_buffer_limits(&wgpu::Limits { max_buffer_size: 0, ..wgpu::Limits::default() }));
        assert!(usable_buffer_limits(&wgpu::Limits::default()));
    }
}
