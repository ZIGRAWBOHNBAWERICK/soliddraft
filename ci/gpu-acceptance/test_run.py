"""Failures must not accept an enumerated, software, fallback or unknown adapter."""

from pathlib import Path
import tempfile
import unittest

from run import require_hardware, selected_native


class AdapterGateTests(unittest.TestCase):
    def test_native_uses_selected_adapter_not_enumerated_hardware(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'render.log'
            log.write_text(
                '[INFO egui_wgpu] Adapter: backend: Vulkan, device_type: DiscreteGpu, name: "NVIDIA"\n'
                '[WARN egui_wgpu] Software rasterizer detected - loss of performance expected. '
                'backend: Vulkan, device_type: Cpu, name: "llvmpipe"\n', encoding='utf-8')
            adapter = selected_native(log)
            self.assertEqual(adapter['name'], '"llvmpipe"')
            with self.assertRaisesRegex(ValueError, 'Software or fallback'):
                require_hardware(adapter, {'Vulkan'})

    def test_browser_rejects_forced_software_even_if_reported_as_non_fallback(self):
        for adapter in (
            {'backend': 'WebGPU', 'vendor': 'google', 'architecture': 'swiftshader', 'fallback': False},
            {'backend': 'WebGPU', 'vendor': 'nvidia', 'fallback': True},
        ):
            with self.subTest(adapter=adapter), self.assertRaisesRegex(ValueError, 'Software or fallback'):
                require_hardware(adapter, {'WebGPU'})

    def test_browser_rejects_missing_adapter_identity_and_webgl(self):
        for adapter in (None, {'backend': 'WebGPU', 'fallback': False},
                        {'backend': 'WebGPU', 'vendor': 'nvidia', 'fallback': None},
                        {'backend': 'webgl2', 'description': 'NVIDIA'}):
            with self.subTest(adapter=adapter), self.assertRaises(ValueError):
                require_hardware(adapter, {'WebGPU'})

    def test_browser_accepts_identified_hardware(self):
        require_hardware({'backend': 'WebGPU', 'vendor': 'nvidia', 'fallback': False}, {'WebGPU'})
        require_hardware({'backend': 'WebGPU', 'vendor': 'apple', 'architecture': 'apple-m3',
                          'fallback': None}, {'WebGPU'})


if __name__ == '__main__':
    unittest.main()
