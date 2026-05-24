import { createBetaStudySession } from './index';

type JsonRecord = Record<string, unknown>;

const hostname = Bun.env.HOST ?? '127.0.0.1';
const path = Bun.env.BETA_STUDY_STORE ?? '.tmp/beta-study/store.json';
const study = await createBetaStudySession({ path });
await study.start();

const server = Bun.serve({
  hostname,
  port: Number(Bun.env.PORT ?? 4174),
  async fetch(request) {
    const url = new URL(request.url);

    if (request.method === 'GET' && url.pathname === '/') {
      return new Response(Bun.file(new URL('./index.html', import.meta.url)), {
        headers: { 'content-type': 'text/html; charset=utf-8' },
      });
    }

    if (request.method === 'GET' && url.pathname === '/state') {
      return json(await study.view());
    }

    if (request.method === 'POST' && url.pathname === '/source') {
      return json(await study.addSource(readSource(await readJson(request))));
    }

    if (request.method === 'POST' && url.pathname === '/generate') {
      return json(await study.generate());
    }

    if (request.method === 'POST' && url.pathname === '/approve') {
      return json(await study.approveDraft(readString(await readJson(request), 'draftId')));
    }

    if (request.method === 'POST' && url.pathname === '/reveal') {
      return json(await study.reveal());
    }

    if (request.method === 'POST' && url.pathname === '/answer') {
      const payload = await readJson(request);
      return json(
        await study.submitAnswer(
          readString(payload, 'answer'),
          readNonNegativeNumber(payload, 'responseTimeMs'),
        ),
      );
    }

    if (request.method === 'POST' && url.pathname === '/next') {
      return json(await study.next());
    }

    return new Response('Not found', { status: 404 });
  },
});

console.log(`Beta study listening on http://${hostname}:${server.port}`);
setInterval(() => {}, 60_000);

function json(value: unknown, status = 200): Response {
  return Response.json(value, {
    status,
    headers: { 'cache-control': 'no-store' },
  });
}

async function readJson(request: Request): Promise<JsonRecord> {
  const value = await request.json();
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error('Request body must be an object');
  }
  return value as JsonRecord;
}

function readSource(payload: JsonRecord): { id: string; title: string; body: string } {
  return {
    id: readString(payload, 'id'),
    title: readString(payload, 'title'),
    body: readString(payload, 'body'),
  };
}

function readString(payload: JsonRecord, key: string): string {
  const value = payload[key];
  if (typeof value !== 'string' || value.trim().length === 0) {
    throw new Error(`${key} must be a non-empty string`);
  }
  return value;
}

function readNonNegativeNumber(payload: JsonRecord, key: string): number {
  const value = payload[key];
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    throw new Error(`${key} must be a non-negative number`);
  }
  return value;
}
