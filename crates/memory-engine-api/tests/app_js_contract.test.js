import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const script = readFileSync(new URL("../assets/app.js", import.meta.url), "utf8");
const HANDOFF_KEY = "memory-engine.submit-handoff.v1";

function browserHarness(options = {}) {
  const documentEvents = new Map();
  const windowEvents = new Map();
  const storage = options.storage ?? new Map();
  const timers = new Map();
  const fetches = [];
  const rootAttributes = new Set();
  let nextTimerId = 1;
  let performanceTraceInput = null;
  let prevented = 0;
  // A monotonic clock that advances by `tick` on every read. Real browsers
  // never return the same perf.now() value twice across a submit-to-landing
  // trace; `tick: 0` (the default) keeps every pre-existing exact-value
  // assertion unchanged, and `tick: 1` lets a test prove elapsed time is
  // genuinely simulated end to end rather than frozen.
  let clock = options.now ?? 100;
  const tick = options.tick ?? 0;
  // requestAnimationFrame is queued, never fired inline: one call to
  // runAnimationFrame() below drains exactly the callbacks queued *before*
  // that call, matching the browser's one-callback-per-paint semantics. A
  // nested requestAnimationFrame scheduled while draining queues for the
  // *next* runAnimationFrame(), never the current one.
  let frameQueue = [];
  let verdictPresent = options.verdict ?? false;
  const sseHandlers = new Map();
  const navigations = [];
  const eventSource = options.eventSource;
  if (eventSource) {
    eventSource.addEventListener = (name, handler) => {
      const handlers = sseHandlers.get(name) ?? [];
      handlers.push(handler);
      sseHandlers.set(name, handlers);
    };
  }

  const addListener = (listeners, name, handler) => {
    const handlers = listeners.get(name) ?? [];
    handlers.push(handler);
    listeners.set(name, handlers);
  };
  const classes = (...names) => ({ contains: (name) => names.includes(name) });
  const controlAttributes = new Map();
  const control = {
    classList: classes(...(options.controlClasses ?? ["me-choice"])),
    textContent: options.controlLabel ?? "Answer",
    name: options.controlName ?? "answer",
    value: options.controlValue ?? options.controlLabel ?? "Answer",
    setAttribute: (name, value) => controlAttributes.set(name, value ?? ""),
    getAttribute: (name) => {
      if (controlAttributes.has(name)) return controlAttributes.get(name);
      if (name === "name") return control.name;
      if (name === "value") return control.value;
      return null;
    },
    removeAttribute: (name) => controlAttributes.delete(name),
  };
  const responseInput = { value: "" };
  const formFields = options.formFields ?? {
    csrfToken: "csrf-test",
    reviewUnitId: "unit-1",
    responseTimeMs: "",
    idempotencyKey: "review-unit-1-0",
  };
  let nativeSubmitCount = 0;
  let viewHtml = options.viewHtml ?? '<p class="me-prompt">Q</p>';
  let dueText = options.dueText ?? "1 due";
  let footerHtml = options.footerHtml ?? '<nav class="me-nav">Home</nav>';
  const fallbackFields = [];
  const form = {
    tagName: "FORM",
    classList: classes(options.formClass ?? "me-choices-form"),
    action: options.action ?? "/app/submit",
    getAttribute: (name) => (name === "action" ? options.action ?? "/app/submit" : null),
    querySelector(selector) {
      if (selector === 'input[name="responseTimeMs"]') return responseInput;
      if (selector === 'input[name="performanceTraceId"]') return performanceTraceInput;
      if (selector === 'button[type="submit"], button:not([type])') return control;
      const fallbackMatch = selector.match(
        /^input\[type="hidden"\]\[name="([^"]+)"\]\[data-scry-fallback="1"\]$/,
      );
      if (fallbackMatch) {
        return fallbackFields.find((field) => field.name === fallbackMatch[1]) ?? null;
      }
      return null;
    },
    querySelectorAll: (selector) => (selector === ".me-choice" ? [control] : []),
    appendChild: (input) => {
      if (input && input.name === "performanceTraceId") {
        performanceTraceInput = input;
        return;
      }
      if (input && input.getAttribute && input.getAttribute("data-scry-fallback") === "1") {
        fallbackFields.push({
          name: input.name || input.getAttribute("name"),
          value: input.value || input.getAttribute("value"),
        });
        return;
      }
      performanceTraceInput = input;
    },
    submit() {
      nativeSubmitCount += 1;
    },
  };
  const headMetas = new Map(Object.entries(options.metas ?? {}));
  const document = {
    documentElement: {
      setAttribute: (name) => rootAttributes.add(name),
      removeAttribute: (name) => rootAttributes.delete(name),
      hasAttribute: (name) => rootAttributes.has(name),
    },
    head: {
      appendChild(meta) {
        if (meta && meta.name) headMetas.set(meta.name, meta.content ?? "");
      },
    },
    visibilityState: options.visibilityState ?? "visible",
    addEventListener: (name, handler) => addListener(documentEvents, name, handler),
    querySelector(selector) {
      if (selector === ".me-verdict" && verdictPresent) return {};
      if (selector === ".ae-view") {
        return {
          get innerHTML() {
            return viewHtml;
          },
          set innerHTML(value) {
            viewHtml = String(value);
            verdictPresent = viewHtml.includes("me-verdict");
          },
          querySelector(inner) {
            if (
              inner ===
                'form.me-next button[type="submit"], form.me-next button:not([type])' &&
              viewHtml.includes("me-next")
            ) {
              return { focus() {} };
            }
            return null;
          },
        };
      }
      if (selector === ".me-due") {
        return {
          get textContent() {
            return dueText;
          },
          set textContent(value) {
            dueText = String(value);
          },
        };
      }
      if (selector === "footer.ae-bar") {
        return {
          get innerHTML() {
            return footerHtml;
          },
          set innerHTML(value) {
            footerHtml = String(value);
          },
        };
      }
      const metaMatch = selector.match(/^meta\[name="([^"]+)"\]$/);
      if (metaMatch) {
        const value = headMetas.get(metaMatch[1]);
        if (value === undefined) return null;
        return {
          getAttribute: (name) => (name === "content" ? value : null),
          setAttribute: (name, next) => {
            if (name === "content") headMetas.set(metaMatch[1], next);
          },
          parentNode: {
            removeChild() {
              headMetas.delete(metaMatch[1]);
            },
          },
        };
      }
      return null;
    },
    getElementById: (id) => (id === "me-jobs" ? options.jobsList ?? null : null),
    querySelectorAll(selector) {
      const match = selector.match(/^meta\[name="([^"]+)"\]$/);
      if (!match) return [];
      const value = headMetas.get(match[1]);
      return value === undefined
        ? []
        : [{ getAttribute: () => value }];
    },
    createElement: (tag) => {
      if (tag === "meta") {
        return {
          name: "",
          content: "",
          setAttribute(name, value) {
            if (name === "name") this.name = value;
            if (name === "content") this.content = value;
          },
        };
      }
      const el = {
        name: "",
        value: "",
        attrs: {},
        setAttribute(name, value) {
          this.attrs[name] = value;
          if (name === "name") this.name = value;
          if (name === "value") this.value = value;
        },
        getAttribute(name) {
          return this.attrs[name] ?? null;
        },
      };
      return el;
    },
  };
  const sessionStorage = {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
    removeItem: (key) => storage.delete(key),
  };
  const fetchImpl = options.fetchImpl;
  const enableInPlace = options.inPlace === true;
  class FakeFormData {
    constructor(source) {
      this.map = new Map();
      if (source === form) {
        for (const [key, value] of Object.entries(formFields)) {
          this.map.set(key, value);
        }
        if (responseInput.value !== "") this.map.set("responseTimeMs", responseInput.value);
      }
    }
    append(name, value) {
      this.map.set(name, value);
    }
    has(name) {
      return this.map.has(name);
    }
    get(name) {
      return this.map.has(name) ? this.map.get(name) : null;
    }
    entries() {
      return this.map.entries();
    }
  }
  class FakeDOMParser {
    parseFromString(html) {
      const viewMatch = html.match(/<div class="ae-view">([\s\S]*?)<\/div>/);
      const dueMatch = html.match(/<span class="me-due">([^<]*)<\/span>/);
      const footerMatch = html.match(/<footer class="ae-bar">([\s\S]*?)<\/footer>/);
      const meta = {};
      for (const match of html.matchAll(
        /<meta name="([^"]+)" content="([^"]*)">/g,
      )) {
        meta[match[1]] = match[2];
      }
      const viewHtmlNext = viewMatch ? viewMatch[1] : "";
      return {
        querySelector(selector) {
          if (selector === ".ae-view") {
            return { innerHTML: viewHtmlNext };
          }
          if (selector === ".me-due" && dueMatch) {
            return { textContent: dueMatch[1] };
          }
          if (selector === "footer.ae-bar" && footerMatch) {
            return { innerHTML: footerMatch[1] };
          }
          const metaMatch = selector.match(/^meta\[name="([^"]+)"\]$/);
          if (metaMatch && meta[metaMatch[1]] !== undefined) {
            return {
              getAttribute: (name) =>
                name === "content" ? meta[metaMatch[1]] : null,
            };
          }
          return null;
        },
      };
    }
  }
  const window = {
    sessionStorage,
    performance: {
      timeOrigin: 1_000_000,
      now: () => {
        const value = clock;
        clock += tick;
        return value;
      },
      getEntriesByType: (type) =>
        type === "navigation" && options.navigation ? [options.navigation] : [],
    },
    crypto: {
      getRandomValues(values) {
        for (let index = 0; index < values.length; index += 1) values[index] = index + 1;
        return values;
      },
    },
    innerWidth: 390,
    Uint8Array,
    addEventListener: (name, handler) => addListener(windowEvents, name, handler),
    requestAnimationFrame: (handler) => {
      frameQueue.push(handler);
      return frameQueue.length;
    },
    fetch(url, request) {
      fetches.push({ url, request });
      if (typeof fetchImpl === "function") {
        // Always re-wrap so the VM sees a real thenable even when the
        // implementation returns a bare value or a foreign-realm Promise.
        return Promise.resolve().then(() => fetchImpl(url, request));
      }
      return Promise.resolve({ catch() {} });
    },
    setTimeout(handler, duration) {
      const id = nextTimerId;
      nextTimerId += 1;
      timers.set(id, { handler, duration });
      return id;
    },
    clearTimeout: (id) => timers.delete(id),
    location: { assign: (url) => navigations.push(url) },
  };
  if (enableInPlace) {
    window.FormData = FakeFormData;
    window.DOMParser = FakeDOMParser;
  }
  window.window = window;
  if (eventSource) window.EventSource = function EventSource() { return eventSource; };

  vm.runInNewContext(script, {
    console,
    document,
    window,
    EventSource: eventSource ? window.EventSource : undefined,
    URL,
    Promise,
    setTimeout,
    clearTimeout,
  });

  return {
    dispatchSubmit() {
      const event = {
        target: form,
        submitter: control,
        preventDefault() {
          prevented += 1;
        },
      };
      for (const handler of documentEvents.get("submit") ?? []) handler(event);
    },
    controlLabel: () => control.textContent,
    controlAttr: (name) => controlAttributes.get(name),
    viewHtml: () => viewHtml,
    dueText: () => dueText,
    footerHtml: () => footerHtml,
    fallbackFields: () => fallbackFields.slice(),
    nativeSubmits: () => nativeSubmitCount,
    responseTimeMs: () => responseInput.value,
    meta: (name) => headMetas.get(name) ?? null,
    dispatchWindow(name, event = {}) {
      for (const handler of windowEvents.get(name) ?? []) handler(event);
    },
    runTimer(duration) {
      const entry = [...timers.entries()].find(([, timer]) => timer.duration === duration);
      if (!entry) throw new Error(`missing ${duration}ms timer`);
      timers.delete(entry[0]);
      entry[1].handler();
    },
    // Drains exactly the callbacks queued before this call — one real frame.
    runAnimationFrame() {
      const queue = frameQueue;
      frameQueue = [];
      for (const handler of queue) handler();
    },
    pendingFrames() {
      return frameQueue.length;
    },
    advanceClock(ms) {
      clock += ms;
    },
    setVerdictPresent(present) {
      verdictPresent = present;
    },
    storage,
    fetches,
    handoff: () => sessionStorage.getItem(HANDOFF_KEY),
    busy: () => document.documentElement.hasAttribute("data-busy"),
    prevented: () => prevented,
    emitJob(job) {
      for (const handler of sseHandlers.get("job") ?? []) {
        handler({ data: JSON.stringify(job) });
      }
    },
    navigations,
  };
}

// Shared fixture: a source document that just submitted a review and left
// the handoff behind in (shared) sessionStorage for a landing document to
// consume, plus the strict request id the server would have rendered.
function submittedHandoff(options = {}) {
  const source = browserHarness(options);
  source.dispatchSubmit();
  const handoff = JSON.parse(source.handoff());
  source.dispatchWindow("pagehide");
  return { source, handoff, requestId: "req_0123456789abcdef0123456789abcdef" };
}

function matchingNavigation(requestId, traceToken) {
  return {
    type: "navigate",
    startTime: 0,
    responseStart: 110,
    responseEnd: 120,
    serverTiming: [
      { name: "request", description: requestId },
      { name: "handoff", description: traceToken },
    ],
  };
}

function matchingLandingOptions(source, handoff, requestId, extra = {}) {
  return {
    storage: source.storage,
    now: 130,
    verdict: true,
    metas: {
      "memory-engine-submit-request": requestId,
      "memory-engine-submit-handoff": handoff.token,
      "memory-engine-csrf-token": "csrf-test",
    },
    navigation: matchingNavigation(requestId, handoff.token),
    ...extra,
  };
}

test("pagehide preserves a native submit handoff for the landing document", () => {
  const browser = browserHarness();
  browser.dispatchSubmit();
  const handoff = browser.handoff();
  expect(handoff).not.toBeNull();

  browser.dispatchWindow("pagehide");

  expect(browser.handoff()).toBe(handoff);
  expect(browser.busy()).toBeFalse();
});

test("busy recovery preserves a slow submit handoff until its TTL", () => {
  const browser = browserHarness();
  browser.dispatchSubmit();
  const handoff = browser.handoff();
  expect(handoff).not.toBeNull();
  expect(browser.busy()).toBeTrue();

  browser.runTimer(30_000);

  expect(browser.handoff()).toBe(handoff);
  expect(browser.busy()).toBeFalse();

  browser.runTimer(65_000);
  expect(browser.handoff()).toBeNull();
});

test("next and content-feedback show pending labels without disabling submitter value", () => {
  const next = browserHarness({
    action: "/app/next",
    formClass: "me-next",
    controlClasses: ["ae-button"],
    controlLabel: "Continue →",
  });
  next.dispatchSubmit();
  expect(next.busy()).toBeTrue();
  expect(next.controlLabel()).toBe("Loading…");
  expect(next.controlAttr("data-pending-label")).toBe("1");
  expect(next.controlAttr("aria-disabled")).toBe("true");

  const feedback = browserHarness({
    action: "/app/content-feedback",
    formClass: "me-content-feedback",
    controlClasses: ["ae-button"],
    controlLabel: "Good question",
  });
  feedback.dispatchSubmit();
  expect(feedback.controlLabel()).toBe("Sending…");

  const choice = browserHarness({
    action: "/app/submit",
    controlClasses: ["me-choice"],
    controlLabel: "42",
  });
  choice.dispatchSubmit();
  expect(choice.controlLabel()).toBe("42");
});

async function flushMicrotasks() {
  for (let i = 0; i < 8; i += 1) await Promise.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

test("review submit fetches and swaps the graded view without inventing a verdict", async () => {
  let resolveFetch;
  const fetchPromise = new Promise((resolve) => {
    resolveFetch = resolve;
  });
  const browser = browserHarness({
    inPlace: true,
    action: "/app/submit",
    controlClasses: ["me-choice"],
    controlLabel: "42",
    controlValue: "42",
    viewHtml: '<p class="me-prompt">What is 6*7?</p><form class="me-choices-form"></form>',
    fetchImpl() {
      return fetchPromise;
    },
  });

  browser.dispatchSubmit();
  expect(browser.prevented()).toBe(1);
  expect(browser.busy()).toBeTrue();
  expect(browser.handoff()).toBeNull();
  expect(browser.controlLabel()).toBe("42");
  expect(browser.fetches).toHaveLength(1);
  expect(browser.fetches[0].url).toBe("/app/submit");
  expect(browser.fetches[0].request.headers["X-Requested-With"]).toBe("scry-inplace");
  const body = browser.fetches[0].request.body;
  expect(body.get("answer")).toBe("42");
  expect(Number(body.get("responseTimeMs"))).toBeGreaterThan(0);

  // Server is the only source of the verdict text.
  resolveFetch({
    ok: true,
    headers: { get: () => "text/html; charset=utf-8" },
    text: () =>
      Promise.resolve(`<!doctype html><html><head>
<meta name="memory-engine-csrf-token" content="csrf-next">
<meta name="memory-engine-submit-request" content="req_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">
</head><body>
<span class="me-due">0 due</span>
<div class="ae-view">
<p class="me-prompt">What is 6*7?</p>
<p class="me-result"><span class="me-verdict">Correct</span></p>
<form class="me-next" action="/app/next" method="post"><button type="submit">Continue</button></form>
</div>
<footer class="ae-bar"><p class="me-tagline">tagline</p></footer>
</body></html>`),
  });

  await flushMicrotasks();

  expect(browser.busy()).toBeFalse();
  expect(browser.viewHtml()).toContain('class="me-verdict">Correct<');
  expect(browser.viewHtml()).toContain("me-next");
  expect(browser.dueText()).toBe("0 due");
  expect(browser.footerHtml()).toContain("tagline");
  expect(browser.meta("memory-engine-csrf-token")).toBe("csrf-next");
  expect(browser.nativeSubmits()).toBe(0);
});

test("in-place submit falls back to native form submit when fetch fails", async () => {
  const browser = browserHarness({
    inPlace: true,
    action: "/app/submit",
    controlClasses: ["me-choice"],
    controlLabel: "42",
    controlValue: "42",
    fetchImpl() {
      return Promise.reject(new Error("network down"));
    },
  });

  browser.dispatchSubmit();
  expect(browser.prevented()).toBe(1);
  await flushMicrotasks();
  expect(browser.nativeSubmits()).toBe(1);
  expect(browser.fallbackFields()).toEqual([{ name: "answer", value: "42" }]);
});

test("continue uses in-place fetch and does not write a submit handoff", async () => {
  const browser = browserHarness({
    inPlace: true,
    action: "/app/next",
    formClass: "me-next",
    controlClasses: ["ae-button"],
    controlLabel: "Continue →",
    fetchImpl() {
      return Promise.resolve({
        ok: true,
        headers: { get: () => "text/html; charset=utf-8" },
        text: () =>
          Promise.resolve(
            `<div class="ae-screen"><span class="me-due">1 due</span><div class="ae-view"><p class="me-prompt">Next card</p></div><footer class="ae-bar"><p class="me-tagline">review</p></footer></div>`,
          ),
      });
    },
  });

  browser.dispatchSubmit();
  expect(browser.prevented()).toBe(1);
  expect(browser.controlLabel()).toBe("Loading…");
  expect(browser.handoff()).toBeNull();
  await flushMicrotasks();
  expect(browser.viewHtml()).toContain("Next card");
  expect(browser.footerHtml()).toContain("review");
  expect(browser.busy()).toBeFalse();
});
test("every native form keeps instant acknowledgment and duplicate suppression", () => {
  const browser = browserHarness({ action: "/app/next" });
  browser.dispatchSubmit();
  expect(browser.busy()).toBeTrue();
  expect(browser.handoff()).toBeNull();

  browser.dispatchSubmit();
  expect(browser.prevented()).toBe(1);

  browser.runTimer(30_000);
  expect(browser.busy()).toBeFalse();
  browser.dispatchSubmit();
  expect(browser.prevented()).toBe(1);
});

test("a second document consumes the handoff and emits one visible receipt only after two real animation frames", () => {
  const { source, handoff, requestId } = submittedHandoff({ tick: 1 });
  const landing = browserHarness(matchingLandingOptions(source, handoff, requestId, { tick: 1 }));

  landing.dispatchWindow("pageshow");
  // The emission is genuinely deferred: nothing has fired yet.
  expect(landing.fetches).toHaveLength(0);
  expect(landing.pendingFrames()).toBe(1);

  landing.runAnimationFrame();
  // First frame only queues the second — still nothing emitted.
  expect(landing.fetches).toHaveLength(0);
  expect(landing.pendingFrames()).toBe(1);

  landing.runAnimationFrame();
  expect(landing.handoff()).toBeNull();
  expect(landing.fetches).toHaveLength(1);
  expect(landing.fetches[0].url).toBe("/app/performance/submit");
  const payload = JSON.parse(landing.fetches[0].request.body);
  expect(payload).toMatchObject({
    requestId,
    traceId: handoff.token,
    viewport: "mobile",
  });
  // With a ticking clock the ack and visible durations are genuinely
  // simulated elapsed time, not the frozen zero a static clock would give.
  expect(payload.tapToAckMs).toBeGreaterThan(0);
  expect(payload.gradedVisibleMs).toBeGreaterThan(payload.tapToAckMs);
  expect(payload.requestToResponseMs + payload.transferMs + payload.navigationMs).toBe(
    payload.gradedVisibleMs
  );
});

test("graded-visible telemetry waits two real animation frames before emitting", () => {
  const { source, handoff, requestId } = submittedHandoff();
  const landing = browserHarness(matchingLandingOptions(source, handoff, requestId));

  landing.dispatchWindow("pageshow");
  expect(landing.pendingFrames()).toBe(1);
  expect(landing.fetches).toHaveLength(0);

  landing.runAnimationFrame();
  expect(landing.pendingFrames()).toBe(1);
  expect(landing.fetches).toHaveLength(0);

  landing.runAnimationFrame();
  expect(landing.pendingFrames()).toBe(0);
  expect(landing.fetches).toHaveLength(1);
});

test("a rejected landing consumes the handoff without emitting telemetry", () => {
  const source = browserHarness();
  source.dispatchSubmit();
  source.dispatchWindow("pagehide");

  const landing = browserHarness({ storage: source.storage, now: 130 });
  landing.dispatchWindow("pageshow");

  expect(landing.handoff()).toBeNull();
  expect(landing.fetches).toHaveLength(0);
});

function entryFormHarness() {
  const formEvents = new Map();
  const button = { disabled: false, textContent: "Get started" };
  const status = { textContent: "" };
  const form = {
    tagName: "FORM",
    classList: { contains: (name) => name === "me-entry-form" },
    getAttribute: (name) => (name === "action" ? "/app/account" : null),
    addEventListener: (name, handler) => {
      const handlers = formEvents.get(name) ?? [];
      handlers.push(handler);
      formEvents.set(name, handlers);
    },
    querySelector: (selector) => {
      if (selector === 'button[type="submit"]') return button;
      if (selector === ".me-entry-status") return status;
      return null;
    },
  };
  const document = {
    documentElement: {
      setAttribute() {},
      removeAttribute() {},
      hasAttribute: () => false,
    },
    addEventListener: () => {},
    querySelector: (selector) => (selector === "form.me-entry-form" ? form : null),
    querySelectorAll: () => [],
    createElement: () => ({ setAttribute() {}, value: "" }),
  };
  const window = {
    sessionStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    performance: { timeOrigin: 0, now: () => 0, getEntriesByType: () => [] },
    crypto: { getRandomValues: (values) => values },
    innerWidth: 390,
    Uint8Array,
    addEventListener: () => {},
    requestAnimationFrame: (handler) => handler(),
    fetch: () => ({ catch() {} }),
    setTimeout: () => 0,
    clearTimeout: () => {},
  };
  window.window = window;

  vm.runInNewContext(script, {
    console,
    document,
    window,
    EventSource: undefined,
    URL,
  });

  return {
    submit: () => {
      for (const handler of formEvents.get("submit") ?? []) handler({});
    },
    button,
    status,
  };
}

test("entry request announces a pending state synchronously, before the network settles", async () => {
  const browser = entryFormHarness();
  let networkSettled = false;
  const network = new Promise((resolve) => {
    setTimeout(() => {
      networkSettled = true;
      resolve();
    }, 150);
  });

  const started = performance.now();
  browser.submit();
  const elapsed = performance.now() - started;

  expect(elapsed).toBeLessThan(100);
  expect(browser.button.disabled).toBe(true);
  expect(browser.button.textContent).toBe("Checking…");
  expect(browser.status.textContent).toBe("Checking…");
  expect(networkSettled).toBe(false);

  await network;
  expect(networkSettled).toBe(true);
});

test("entry request enhancement is a no-op once the button is pending", () => {
  const browser = entryFormHarness();
  browser.submit();
  browser.button.textContent = "Checking… (server response pending)";
  browser.submit();
  expect(browser.button.textContent).toBe("Checking… (server response pending)");
});

test("a BFCache-restored landing clears the handoff and never schedules emission", () => {
  const { source, handoff, requestId } = submittedHandoff();
  const landing = browserHarness(matchingLandingOptions(source, handoff, requestId));

  landing.dispatchWindow("pageshow", { persisted: true });

  expect(landing.handoff()).toBeNull();
  expect(landing.busy()).toBeFalse();
  expect(landing.fetches).toHaveLength(0);
  expect(landing.pendingFrames()).toBe(0);
});

test("a two-RAF emission already queued before pagehide is invalidated, not fired stale after BFCache restore", () => {
  const { source, handoff, requestId } = submittedHandoff();
  const landing = browserHarness(matchingLandingOptions(source, handoff, requestId));

  landing.dispatchWindow("pageshow");
  expect(landing.pendingFrames()).toBe(1);

  // The tab is hidden and this document is frozen into BFCache before the
  // first animation frame ever paints.
  landing.dispatchWindow("pagehide");
  landing.dispatchWindow("pageshow", { persisted: true });

  // The already-queued frame callback (queued before pagehide) finally
  // fires on resume. It must revalidate and refuse to schedule the second
  // frame or emit anything — the landing it was scheduled for is stale.
  landing.runAnimationFrame();
  expect(landing.pendingFrames()).toBe(0);
  expect(landing.fetches).toHaveLength(0);
});

test("a landing whose verdict disappears before the second frame never emits", () => {
  const { source, handoff, requestId } = submittedHandoff();
  const landing = browserHarness(matchingLandingOptions(source, handoff, requestId));

  landing.dispatchWindow("pageshow");
  expect(landing.pendingFrames()).toBe(1);

  // Document state changed between scheduling and the first frame (e.g. a
  // failed re-render). The queued callback must revalidate, not trust the
  // snapshot it captured at schedule time.
  landing.setVerdictPresent(false);
  landing.runAnimationFrame();

  expect(landing.pendingFrames()).toBe(0);
  expect(landing.fetches).toHaveLength(0);
});

test("an expired handoff at landing time is never scheduled for emission", () => {
  const { source, handoff, requestId } = submittedHandoff();
  // now (130000ms past timeOrigin via `now`) is well beyond startedAtMs +
  // the 65s handoff TTL.
  const landing = browserHarness(
    matchingLandingOptions(source, handoff, requestId, { now: 70_000 })
  );

  landing.dispatchWindow("pageshow");

  expect(landing.handoff()).toBeNull();
  expect(landing.fetches).toHaveLength(0);
  expect(landing.pendingFrames()).toBe(0);
});

test("a landing whose meta request id does not match the rendered Server-Timing never emits", () => {
  const { source, handoff, requestId } = submittedHandoff();
  const mismatchedRequestId = "req_ffffffffffffffffffffffffffffffff";
  const landing = browserHarness(
    matchingLandingOptions(source, handoff, requestId, {
      metas: {
        "memory-engine-submit-request": mismatchedRequestId,
        "memory-engine-submit-handoff": handoff.token,
        "memory-engine-csrf-token": "csrf-test",
      },
    })
  );

  landing.dispatchWindow("pageshow");

  expect(landing.handoff()).toBeNull();
  expect(landing.fetches).toHaveLength(0);
  expect(landing.pendingFrames()).toBe(0);
});

test("a landing document without Server-Timing on its own navigation entry never emits", () => {
  const { source, handoff, requestId } = submittedHandoff();
  const landing = browserHarness(
    matchingLandingOptions(source, handoff, requestId, {
      navigation: { type: "navigate", startTime: 0, responseStart: 110, responseEnd: 120 },
    })
  );

  landing.dispatchWindow("pageshow");

  expect(landing.handoff()).toBeNull();
  expect(landing.fetches).toHaveLength(0);
  expect(landing.pendingFrames()).toBe(0);
});

test("a second pageshow on the same landing document never emits a duplicate receipt", () => {
  const { source, handoff, requestId } = submittedHandoff();
  const landing = browserHarness(matchingLandingOptions(source, handoff, requestId));

  landing.dispatchWindow("pageshow");
  landing.runAnimationFrame();
  landing.runAnimationFrame();
  expect(landing.fetches).toHaveLength(1);

  landing.dispatchWindow("pageshow");
  expect(landing.pendingFrames()).toBe(0);
  expect(landing.fetches).toHaveLength(1);
});


function jobsListHarness() {
  const meta = { textContent: "old" };
  const row = {
    dataset: { jobId: "job-1" },
    querySelector: (selector) => (selector === ".me-job-meta" ? meta : null),
  };
  return {
    meta,
    querySelector: () => row,
    insertBefore() {},
  };
}

test("SSE patches intermediate jobs but refreshes the workspace on success", () => {
  const eventSource = {};
  const list = jobsListHarness();
  const browser = browserHarness({ eventSource, jobsList: list });

  browser.emitJob({ id: "job-1", status: "running" });
  expect(list.meta.textContent).toBe("Generating cards…");
  expect(browser.navigations).toEqual([]);

  browser.emitJob({ id: "job-1", status: "succeeded" });
  expect(browser.navigations).toEqual(["/"]);
});

test("SSE refreshes the workspace on failure for authoritative retry controls", () => {
  const eventSource = {};
  const list = jobsListHarness();
  const browser = browserHarness({ eventSource, jobsList: list });

  browser.emitJob({ id: "job-1", status: "failed", error: "provider unavailable" });
  expect(list.meta.textContent).toBe("provider unavailable");
  expect(browser.navigations).toEqual(["/"]);
});

test("SSE terminal events never navigate away from pages without the jobs surface", () => {
  const eventSource = {};
  const browser = browserHarness({ eventSource, jobsList: null });

  browser.emitJob({ id: "job-1", status: "succeeded" });
  browser.emitJob({ id: "job-1", status: "failed", error: "provider unavailable" });
  expect(browser.navigations).toEqual([]);
});
