// Installed before the shipping Wasm app boots, only in the loopback gate server.
// Associate the canvas's configured device with its own adapter, not a separate probe.
(() => {
  const adapters = new WeakMap();
  const devices = new WeakMap();
  const force = new URLSearchParams(location.search).has('force-software');
  const state = window.gpuAcceptance = { renderer: null, requests: [], document: null };
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'document') state.document = detail;
  });
  state.painted = () => new Promise(resolve => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve(true)));
  });
  const canvasId = 'soliddraft';

  if (navigator.gpu) {
    const requestAdapter = navigator.gpu.requestAdapter.bind(navigator.gpu);
    Object.defineProperty(navigator.gpu, 'requestAdapter', {value: async options => {
      const adapter = await requestAdapter(force ? {...options, forceFallbackAdapter: true} : options);
      if (!adapter) {
        state.requests.push(null);
        return adapter;
      }
      const info = adapter.info;
      const record = {
        backend: 'WebGPU',
        fallback: info?.isFallbackAdapter ?? adapter.isFallbackAdapter ?? null,
        vendor: info?.vendor ?? '', architecture: info?.architecture ?? '',
        device: info?.device ?? '', description: info?.description ?? '',
      };
      state.requests.push(record);
      adapters.set(adapter, record);
      return adapter;
    }});
    const requestDevice = GPUAdapter.prototype.requestDevice;
    GPUAdapter.prototype.requestDevice = async function (...args) {
      const device = await requestDevice.apply(this, args);
      devices.set(device, adapters.get(this));
      return device;
    };
  }
  if (window.GPUCanvasContext) {
    const configure = GPUCanvasContext.prototype.configure;
    GPUCanvasContext.prototype.configure = function (options) {
      configure.call(this, options);
      if (this.canvas.id === canvasId) {
        state.renderer = devices.get(options.device) ?? {backend: 'WebGPU'};
      }
    };
  }
  const getContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (kind, ...args) {
    const context = getContext.call(this, kind, ...args);
    if (context && this.id === canvasId && ['webgl', 'webgl2'].includes(kind)) {
      const debug = context.getExtension('WEBGL_debug_renderer_info');
      state.renderer = {backend: kind, description: debug && context.getParameter(debug.UNMASKED_RENDERER_WEBGL)};
    }
    return context;
  };

  // Same File/DataTransfer input as dropping a local part on the viewport.
  state.drop = async name => {
    const response = await fetch(`/.gpu-acceptance/${name}.dwprt`);
    if (!response.ok) throw new Error(`Could not load ${name}: ${response.status}`);
    const file = new File([await response.blob()], `${name}.dwprt`);
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(file);
    document.getElementById(canvasId).dispatchEvent(new DragEvent('drop', {bubbles: true, dataTransfer}));
  };
})();
