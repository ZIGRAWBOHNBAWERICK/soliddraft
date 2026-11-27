// Run on a fresh /client/ page through chrome-devtools-axi run at 1400 x 813.
function check(value, message) { if (!value) throw new Error(message); }

await page.eval(async () => {
  const gpu = navigator.gpu;
  const requestAdapter = gpu.requestAdapter;
  const adapter = await requestAdapter.call(gpu);
  if (adapter?.info.architecture !== 'swiftshader') {
    throw new Error(`Expected SwiftShader WebGPU, got ${adapter?.info.architecture ?? 'no adapter'}`);
  }
  const html = await (await fetch('/')).text();
  // Skip only SolidDraft's startup software-adapter probe. wgpu gets the real adapter.
  gpu.requestAdapter = () => {
    delete gpu.requestAdapter;
    return Promise.resolve(null);
  };
  const devices = new WeakMap();
  const requestDevice = GPUAdapter.prototype.requestDevice;
  GPUAdapter.prototype.requestDevice = async function (options) {
    const device = await requestDevice.call(this, options);
    devices.set(device, this.info.architecture);
    return device;
  };
  const configure = GPUCanvasContext.prototype.configure;
  GPUCanvasContext.prototype.configure = function (options) {
    configure.call(this, options);
    if (this.canvas.id === 'soliddraft') window.sdWebGpu = devices.get(options.device);
  };
  // Some SwiftShader builds accept this allocation. Reproduce the reported driver
  // rejection for the body uniform while leaving all other WebGPU calls intact.
  const createBuffer = GPUDevice.prototype.createBuffer;
  GPUDevice.prototype.createBuffer = function (descriptor) {
    if (devices.get(this) === 'swiftshader' && descriptor.label === 'body' && descriptor.mappedAtCreation) {
      const message = `createBuffer failed, size (${descriptor.size}) is too large for the implementation when mappedAtCreation == true`;
      window.sdMappedBodyError = message;
      throw new RangeError(message);
    }
    return createBuffer.call(this, descriptor);
  };
  history.replaceState(null, '', '/');
  document.open();
  document.write(html);
  document.close();
  document.addEventListener('soliddraft-feedback', event => {
    const detail = JSON.parse(event.detail);
    if (detail.type === 'document') window.sdPart = detail;
  });
  return true;
});
await page.wait('#status.hidden', 60000);
check(await page.eval(() => window.sdWebGpu === 'swiftshader'), 'SolidDraft did not configure its canvas on SwiftShader WebGPU');

const shown = async name => {
  const drawn = await page.eval(`window.wasmBindings.${name}()`);
  return drawn ? JSON.parse(drawn) : null;
};
async function until(read, ready, what) {
  let value;
  for (let i = 0; i < 300; i++) {
    const error = await page.eval(() =>
      window.sdMappedBodyError ?? document.querySelector('#status.shown')?.textContent ?? null);
    check(!error, error);
    value = await read();
    if (ready(value)) return value;
    await page.wait(100);
  }
  throw new Error(`${what}: ${JSON.stringify(value)}`);
}
const control = async name => until(
  async () => (await shown('shown_controls'))?.[name], Boolean, `Missing control ${name}`);
const part = () => page.eval(() => window.sdPart ?? null);
const click = (x, y) => page.eval(`async () => {
  const canvas = document.querySelector('#soliddraft');
  const at = { clientX: ${x}, clientY: ${y}, bubbles: true, cancelable: true,
    pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0 };
  canvas.dispatchEvent(new PointerEvent('pointermove', at));
  canvas.dispatchEvent(new PointerEvent('pointerdown', { ...at, buttons: 1 }));
  await new Promise(resolve => setTimeout(resolve, 50));
  canvas.dispatchEvent(new PointerEvent('pointerup', at));
  return true;
}`);
const clickControl = async name => {
  const { x, y } = await control(name);
  await click(x, y);
};

check(await page.eval(() => innerWidth === 1400 && innerHeight === 813), 'Set a 1400 x 813 viewport');
await click(124, 143); // New part on the start page.
await until(part, doc => doc?.extension === '.dwprt' && doc.count === 0, 'New part did not open');
await click(222, 95); // Extrude in the Solid strip.
await clickControl('Rectangle');
await clickControl('Form OK');
await until(part, doc => doc?.extension === '.dwprt' && doc.count === 1, 'Extrude did not add a feature');
await control('Tree: Boss-Extrude1');
await until(() => shown('shown_viewport'), view => view?.triangles > 0 && !view.pending && !view.preview,
  'The committed extrusion did not draw');
console.log('SwiftShader WebGPU created Boss-Extrude1');
