import { endpoint, sitekey, support } from "./config.mjs";

let turnstileLoading;
function loadTurnstile() {
  if (globalThis.turnstile) return Promise.resolve(globalThis.turnstile);
  if (!turnstileLoading) {
    turnstileLoading = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src =
        "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
      script.onload = () => resolve(globalThis.turnstile);
      script.onerror = () => {
        script.remove();
        turnstileLoading = undefined;
        reject(new Error("Spam check could not load."));
      };
      document.head.append(script);
    });
  }
  return turnstileLoading;
}

export function mountFeedback(
  root,
  { source, version, commit = "unknown", context = () => null },
) {
  root.innerHTML = `
    <form class="feedback-form">
      <fieldset class="feedback-fields">
        <label>What would you like to share?
          <select name="kind"><option value="bug">A bug</option><option value="idea">An idea</option><option value="other">Something else</option></select>
        </label>
        <label>Your message <textarea name="message" rows="5" minlength="5" maxlength="4000" required placeholder="What happened? What did you expect?"></textarea></label>
        <label>Email (optional) <input name="email" type="email" maxlength="254" autocomplete="email" placeholder="If you would like us to contact you"></label>
        <p>Do not include confidential details or files. Your feedback goes to a private inbox for Zigrawbohnbawerick Pty Ltd.</p>
      </fieldset>
      <button class="pill feedback-review" type="submit">Review feedback</button>
      <section class="feedback-confirm" hidden aria-label="Review feedback">
        <label class="feedback-check"><input name="omit" type="checkbox"> Leave diagnostic context out</label>
        <p>This is the complete message we will save. A separate Cloudflare spam-check token is sent to verify the request.</p>
        <pre tabindex="0" aria-label="Message to send"></pre>
        <p>Diagnostics use command IDs and error categories, not command arguments, file contents, names or paths. Size classes count features, drawing items or assembly components.</p>
        <div class="feedback-challenge"></div>
        <div class="feedback-buttons"><button class="pill" type="button" data-send disabled>Send feedback</button><button class="pill" type="button" data-edit>Edit message</button></div>
      </section>
      <p role="status" aria-live="polite" class="feedback-status"></p>
    </form>`;
  const form = root.querySelector("form");
  const fields = form.querySelector("fieldset");
  const review = form.querySelector(".feedback-review");
  const confirm = form.querySelector(".feedback-confirm");
  const omit = form.elements.omit;
  const preview = form.querySelector("pre");
  const send = form.querySelector("[data-send]");
  const edit = form.querySelector("[data-edit]");
  const status = form.querySelector("[role=status]");
  let snapshot;
  let payload;
  let token;
  let widget;
  let disposed = false;
  let sendFailed = false;
  let turnstile;
  const updatePreview = () => {
    payload = { ...snapshot, context: omit.checked ? null : snapshot.context };
    preview.textContent = JSON.stringify(payload, null, 2);
  };
  const resetChallenge = () => {
    token = undefined;
    send.disabled = true;
    if (widget !== undefined) turnstile.reset(widget);
  };
  const removeChallenge = () => {
    if (widget !== undefined) turnstile.remove(widget);
    widget = undefined;
    token = undefined;
  };
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!form.reportValidity() || !confirm.hidden) return;
    snapshot = {
      source,
      version,
      commit,
      kind: form.elements.kind.value,
      message: form.elements.message.value,
      context: context(),
    };
    if (form.elements.email.value.trim())
      snapshot.email = form.elements.email.value.trim();
    omit.hidden = snapshot.context === null;
    omit.closest("label").hidden = snapshot.context === null;
    updatePreview();
    fields.disabled = true;
    review.hidden = true;
    confirm.hidden = false;
    status.textContent = "Loading spam check…";
    preview.focus();
    try {
      turnstile = await loadTurnstile();
      if (disposed || confirm.hidden) return;
      status.textContent = "Complete the spam check to send feedback.";
      widget = turnstile.render(form.querySelector(".feedback-challenge"), {
        sitekey,
        action: "feedback",
        size: form.clientWidth < 300 ? "compact" : "flexible",
        theme: "auto",
        callback: (value) => {
          token = value;
          send.disabled = false;
          if (!sendFailed)
            status.textContent =
              "Ready to send. Review the message above first.";
        },
        "expired-callback": () => {
          resetChallenge();
          status.textContent = "Spam check expired. Please complete it again.";
        },
        "error-callback": () => {
          token = undefined;
          send.disabled = true;
          status.textContent = `Spam check failed. Retry the check or email ${support}.`;
        },
      });
    } catch {
      status.textContent = `Spam check could not load. Choose Edit message to retry, or email ${support}.`;
    }
  });
  omit.addEventListener("change", updatePreview);
  edit.addEventListener("click", () => {
    removeChallenge();
    sendFailed = false;
    fields.disabled = false;
    confirm.hidden = true;
    review.hidden = false;
    status.textContent = "";
    form.elements.message.focus();
  });
  send.addEventListener("click", async () => {
    if (!token || send.disabled) return;
    sendFailed = false;
    send.disabled = edit.disabled = omit.disabled = true;
    status.textContent = "Sending feedback…";
    try {
      const response = await fetch(endpoint, {
        method: "POST",
        credentials: "omit",
        referrerPolicy: "no-referrer",
        headers: {
          "Content-Type": "application/json",
          "X-Turnstile-Token": token,
        },
        body: JSON.stringify(payload),
        signal: AbortSignal.timeout(20_000),
      });
      const result = await response.json();
      if (!response.ok)
        throw new Error(result.error || "Feedback was not saved.");
      removeChallenge();
      confirm.hidden = true;
      fields.disabled = false;
      form.reset();
      review.hidden = false;
      status.textContent = `Feedback saved. Thank you. Reference: ${result.id}`;
      review.focus();
    } catch (error) {
      sendFailed = true;
      status.textContent = `${error.message} Your message is still here. Try again or email ${support}.`;
      resetChallenge();
    } finally {
      edit.disabled = omit.disabled = false;
    }
  });
  return () => {
    disposed = true;
    removeChallenge();
    root.replaceChildren();
  };
}
