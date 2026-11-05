import { createRecorder } from "./context.mjs";
import { mountFeedback } from "./form.mjs";
import { support } from "./config.mjs";

const recorder = createRecorder();
const launcher = document.createElement("button");
launcher.id = "feedback-launcher";
launcher.type = "button";
launcher.textContent = "Give feedback";
const offer = document.createElement("aside");
offer.id = "feedback-offer";
offer.hidden = true;
offer.setAttribute("aria-live", "polite");
offer.innerHTML =
  '<p>Something went wrong. Would you like to give feedback?</p><button type="button" data-open>Give feedback</button> <button type="button" data-dismiss>Dismiss</button>';
const dialog = document.createElement("dialog");
dialog.id = "feedback-dialog";
dialog.setAttribute("aria-labelledby", "feedback-heading");
dialog.innerHTML = `<div class="feedback-heading"><h1 id="feedback-heading">Give feedback</h1><button type="button" data-close aria-label="Close feedback">Close</button></div><p>You can also email <a href="mailto:${support}">${support}</a>.</p><div data-feedback-form></div>`;
document.body.append(launcher, offer, dialog);
let unmount;
let offered = false;
function open() {
  if (dialog.open) return;
  offer.hidden = true;
  const { feedbackVersion = "unknown", feedbackCommit = "unknown" } =
    document.documentElement.dataset;
  unmount = mountFeedback(dialog.querySelector("[data-feedback-form]"), {
    source: "app",
    version: feedbackVersion,
    commit: feedbackCommit,
    context: () =>
      recorder.context({
        ua: navigator.userAgent,
        width: screen.width,
        height: screen.height,
      }),
  });
  dialog.showModal();
}
function diagnostic(code, level = "error") {
  recorder.diagnostic(code, level);
  if (level === "error" && !offered) {
    offered = true;
    offer.hidden = false;
  }
}
launcher.addEventListener("click", open);
offer.querySelector("[data-open]").addEventListener("click", open);
offer.querySelector("[data-dismiss]").addEventListener("click", () => {
  offer.hidden = true;
});
dialog
  .querySelector("[data-close]")
  .addEventListener("click", () => dialog.close());
dialog.addEventListener("close", () => {
  unmount?.();
  launcher.focus();
});
window.addEventListener("error", () => diagnostic("browser-error"));
window.addEventListener("unhandledrejection", () =>
  diagnostic("unhandled-rejection"),
);
document.addEventListener("soliddraft-feedback", (event) => {
  try {
    const data = JSON.parse(event.detail);
    if (data.type === "action") recorder.action(data.command);
    if (data.type === "diagnostic") diagnostic(data.code, data.level);
    if (data.type === "document") recorder.document(data.extension, data.count);
  } catch {
    /* Ignore events from unrelated page scripts. */
  }
});
