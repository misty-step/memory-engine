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

  const addListener = (listeners, name, handler) => {
    const handlers = listeners.get(name) ?? [];
    handlers.push(handler);
    listeners.set(name, handlers);
  };
  const classes = (...names) => ({ contains: (name) => names.includes(name) });
  const controlAttributes = new Set();
  const control = {
    classList: classes("me-choice"),
    setAttribute: (name) => controlAttributes.add(name),
    removeAttribute: (name) => controlAttributes.delete(name),
  };
  const responseInput = { value: "" };
  const form = {
    tagName: "FORM",
    classList: classes("me-choices-form"),
    getAttribute: (name) => (name === "action" ? options.action ?? "/app/submit" : null),
    querySelector(selector) {
      if (selector === 'input[name="responseTimeMs"]') return responseInput;
      if (selector === 'input[name="performanceTraceId"]') return performanceTraceInput;
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
    addEventListener: (name, handler) => addListener(documentEvents, name, handler),
    querySelector: (selector) => (selector === ".me-verdict" && options.verdict ? {} : null),
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
      now: () => options.now ?? 100,
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
    requestAnimationFrame: (handler) => handler(),
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
    dispatchWindow(name, event = {}) {
      for (const handler of windowEvents.get(name) ?? []) handler(event);
    },
    runTimer(duration) {
      const entry = [...timers.entries()].find(([, timer]) => timer.duration === duration);
      if (!entry) throw new Error(`missing ${duration}ms timer`);
      timers.delete(entry[0]);
      entry[1].handler();
    },
    storage,
    fetches,
    handoff: () => sessionStorage.getItem(HANDOFF_KEY),
    busy: () => document.documentElement.hasAttribute("data-busy"),
    prevented: () => prevented,
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

test("a second document consumes the handoff and emits one visible receipt", () => {
  const source = browserHarness();
  source.dispatchSubmit();
  const handoff = JSON.parse(source.handoff());
  source.dispatchWindow("pagehide");

  const requestId = "req_0123456789abcdef0123456789abcdef";
  const landing = browserHarness({
    storage: source.storage,
    now: 130,
    verdict: true,
    metas: {
      "memory-engine-submit-request": requestId,
      "memory-engine-submit-handoff": handoff.token,
      "memory-engine-csrf-token": "csrf-test",
    },
    navigation: {
      type: "navigate",
      startTime: 0,
      responseStart: 110,
      responseEnd: 120,
      serverTiming: [
        { name: "request", description: requestId },
        { name: "handoff", description: handoff.token },
      ],
    },
  });
  landing.dispatchWindow("pageshow");

  expect(landing.handoff()).toBeNull();
  expect(landing.fetches).toHaveLength(1);
  expect(landing.fetches[0].url).toBe("/app/performance/submit");
  const payload = JSON.parse(landing.fetches[0].request.body);
  expect(payload).toMatchObject({
    requestId,
    traceId: handoff.token,
    tapToAckMs: 0,
    requestToResponseMs: 10,
    transferMs: 10,
    navigationMs: 10,
    gradedVisibleMs: 30,
    viewport: "mobile",
  });
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
