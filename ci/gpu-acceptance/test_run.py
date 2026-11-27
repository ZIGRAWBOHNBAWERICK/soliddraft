"""Failures must not accept an enumerated, software, fallback or unknown adapter, or blame the app for a hidden page."""

from pathlib import Path
import plistlib
import tempfile
import unittest

from run import console_unlocked, require_hardware, selected_native, wait_drawn


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


class BrowserSessionTests(unittest.TestCase):
    """A hidden page never draws, so the gate must say so instead of timing out on the app."""

    @staticmethod
    def registry(*sessions):
        return plistlib.dumps({'IOConsoleLocked': False, 'IOConsoleUsers': list(sessions)})

    def test_locked_session_is_refused(self):
        unlocked = {'kCGSSessionUserIDKey': 501, 'kCGSSessionOnConsoleKey': True}
        locked = {**unlocked, 'CGSSessionScreenIsLocked': True, 'CGSSessionScreenLockedTime': 1795789773}
        self.assertTrue(console_unlocked(self.registry(unlocked), 501))
        self.assertFalse(console_unlocked(self.registry(locked), 501))
        # Another user's unlocked session on the console does not make this user's Safari visible.
        self.assertFalse(console_unlocked(self.registry({**unlocked, 'kCGSSessionUserIDKey': 502}), 501))
        self.assertFalse(console_unlocked(self.registry({**unlocked, 'kCGSSessionOnConsoleKey': False}), 501))
        self.assertFalse(console_unlocked(self.registry(), 501))

    def test_timeout_on_a_hidden_page_names_the_hidden_page(self):
        # The Safari 27 record from a locked Mac: started, overlay hidden, canvas never configured.
        record = {'renderer': None, 'document': None, 'ready': True, 'status': None, 'visibility': 'hidden'}

        def viewport():
            return wait_drawn(lambda: record['renderer'] and record['ready'], 'the shipping Wasm viewport', record,
                              seconds=0.1)

        with self.assertRaisesRegex(RuntimeError, 'viewport: the page was hidden'):
            viewport()
        record['visibility'] = 'visible'
        with self.assertRaisesRegex(RuntimeError, r'^Timed out waiting for the shipping Wasm viewport$'):
            viewport()


if __name__ == '__main__':
    unittest.main()
