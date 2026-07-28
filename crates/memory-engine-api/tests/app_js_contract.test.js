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
    setAttribute: (name, value) => controlAttributes.set(name, value ?? ""),
    getAttribute: (name) =>
      controlAttributes.has(name) ? controlAttributes.get(name) : null,
    removeAttribute: (name) => controlAttributes.delete(name),
  };
  const responseInput = { value: "" };
  const form = {
    tagName: "FORM",
    classList: classes(options.formClass ?? "me-choices-form"),
    getAttribute: (name) => (name === "action" ? options.action ?? "/app/submit" : null),
    querySelector(selector) {
      if (selector === 'input[name="responseTimeMs"]') return responseInput;
      if (selector === 'input[name="performanceTraceId"]') return performanceTraceInput;
      if (selector === 'button[type="submit"], button:not([type])') return control;
      return null;
    },
    querySelectorAll: (selector) => (selector === ".me-choice" ? [control] : []),
    appendChild: (input) => {
      performanceTraceInput = input;
    },
  };
  const document = {
    documentElement: {
      setAttribute: (name) => rootAttributes.add(name),
      removeAttribute: (name) => rootAttributes.delete(name),
      hasAttribute: (name) => rootAttributes.has(name),
    },
    visibilityState: options.visibilityState ?? "visible",
    addEventListener: (name, handler) => addListener(documentEvents, name, handler),
    querySelector: (selector) => (selector === ".me-verdict" && verdictPresent ? {} : null),
    getElementById: (id) => (id === "me-jobs" ? options.jobsList ?? null : null),
    querySelectorAll(selector) {
      const match = selector.match(/^meta\[name="([^"]+)"\]$/);
      const value = match ? options.metas?.[match[1]] : undefined;
      return value === undefined ? [] : [{ getAttribute: () => value }];
    },
    createElement: () => ({ setAttribute() {}, value: "" }),
  };
  const sessionStorage = {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
    removeItem: (key) => storage.delete(key),
  };
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
      return { catch() {} };
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
  window.window = window;
  if (eventSource) window.EventSource = function EventSource() { return eventSource; };

  vm.runInNewContext(script, {
    console,
    document,
    window,
    EventSource: eventSource ? window.EventSource : undefined,
    URL,
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

function waitlistFormHarness() {
  const formEvents = new Map();
  const button = { disabled: false, textContent: "Join the waitlist" };
  const status = { textContent: "" };
  const form = {
    tagName: "FORM",
    classList: { contains: (name) => name === "me-waitlist-form" },
    getAttribute: (name) => (name === "action" ? "/app/waitlist" : null),
    addEventListener: (name, handler) => {
      const handlers = formEvents.get(name) ?? [];
      handlers.push(handler);
      formEvents.set(name, handlers);
    },
    querySelector: (selector) => {
      if (selector === 'button[type="submit"]') return button;
      if (selector === ".me-waitlist-status") return status;
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
    querySelector: (selector) => (selector === "form.me-waitlist-form" ? form : null),
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

test("waitlist join announces a pending state synchronously, before the deferred network settles", async () => {
  const browser = waitlistFormHarness();

  // Stand in for the real native POST: a Postgres-backed join connects and
  // migrates per call (observed 161-700ms TTFB), so model that round trip as
  // a promise that only settles well after the acknowledgment budget. The
  // synchronous submit handler must already have updated the DOM before this
  // "network" is even given a chance to run, since JS never preempts a
  // running handler to service a timer.
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
  expect(browser.button.textContent).toBe("Joining…");
  expect(browser.status.textContent).toBe("Joining…");
  expect(networkSettled).toBe(false);

  await network;
  expect(networkSettled).toBe(true);
});

test("waitlist join enhancement is a no-op once the button is already pending", () => {
  const browser = waitlistFormHarness();
  browser.submit();
  browser.button.textContent = "Joining… (server response pending)";
  browser.submit();
  expect(browser.button.textContent).toBe("Joining… (server response pending)");
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
