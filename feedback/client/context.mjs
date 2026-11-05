const seconds = (ms) =>
  Math.min(31_536_000, Math.max(0, Math.floor(ms / 1000)));
const safeCommand = (value) =>
  typeof value === "string" && /^[a-z][a-z0-9_.-]{0,79}$/.test(value);
const codes = [
  "app-error",
  "app-warning",
  "browser-error",
  "unhandled-rejection",
  "crash",
  "startup-error",
];

export function browserAndOS(ua) {
  const browser =
    /(?:Edg|Firefox|Chrome|Version)\/[\d.]+/.exec(ua)?.[0] ?? "Unknown";
  const os =
    /Windows NT [\d.]+|Android [\d.]+|(?:CPU (?:iPhone )?OS|Mac OS X) [\d_]+|Linux/.exec(
      ua,
    )?.[0] ?? "Unknown";
  return {
    browser: browser.replace("/", " ").replace("Version", "Safari"),
    os: os.replaceAll("_", "."),
  };
}

export function sizeClass(count) {
  if (!Number.isSafeInteger(count) || count < 0) return "unknown";
  return count === 0
    ? "empty"
    : count <= 50
      ? "small"
      : count <= 500
        ? "medium"
        : "large";
}

export function buildContext({
  ua,
  width,
  height,
  now,
  started,
  actions,
  diagnostics,
  document,
}) {
  const recent = (items, count) =>
    items
      .slice(-count)
      .map(({ at, ...item }) => ({ ...item, secondsAgo: seconds(now - at) }));
  return {
    ...browserAndOS(ua),
    screen: [width, height].map((n) =>
      Math.min(100_000, Math.max(0, Math.round(n) || 0)),
    ),
    sessionSeconds: seconds(now - started),
    actions: recent(
      actions
        .filter((a) => safeCommand(a.command))
        .map(({ command, at }) => ({ command, at })),
      20,
    ),
    diagnostics: recent(
      diagnostics
        .filter(
          (d) =>
            codes.includes(d.code) && ["error", "warning"].includes(d.level),
        )
        .map(({ code, level, at }) => ({ code, level, at })),
      10,
    ),
    document: {
      extension: ["none", ".dwprt", ".dwdrw", ".dwasm"].includes(
        document?.extension,
      )
        ? document.extension
        : "none",
      sizeClass: ["empty", "small", "medium", "large", "unknown"].includes(
        document?.sizeClass,
      )
        ? document.sizeClass
        : "unknown",
    },
  };
}

export function createRecorder(clock = () => performance.now()) {
  const started = clock();
  const actions = [];
  const diagnostics = [];
  let document = { extension: "none", sizeClass: "unknown" };
  return {
    action(command) {
      if (safeCommand(command)) {
        actions.push({ command, at: clock() });
        actions.splice(0, Math.max(0, actions.length - 20));
      }
    },
    diagnostic(code, level = "error") {
      if (codes.includes(code) && ["error", "warning"].includes(level)) {
        diagnostics.push({ code, level, at: clock() });
        diagnostics.splice(0, Math.max(0, diagnostics.length - 10));
      }
    },
    document(extension, count) {
      document = { extension, sizeClass: sizeClass(count) };
    },
    context(environment) {
      return buildContext({
        ...environment,
        now: clock(),
        started,
        actions,
        diagnostics,
        document,
      });
    },
  };
}
