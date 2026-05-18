import { createWebShellSession } from './index';

type AnswerPayload = {
  answer: string;
  responseTimeMs: number;
};

const shell = createWebShellSession();
await shell.start();

const hostname = Bun.env.HOST ?? '127.0.0.1';

const server = Bun.serve({
  hostname,
  port: Number(Bun.env.PORT ?? 4173),
  async fetch(request) {
    const url = new URL(request.url);

    if (request.method === 'GET' && url.pathname === '/') {
      return new Response(Bun.file(new URL('./index.html', import.meta.url)), {
        headers: { 'content-type': 'text/html; charset=utf-8' },
      });
    }

    if (request.method === 'GET' && url.pathname === '/state') {
      return json(shell.view());
    }

    if (request.method === 'POST' && url.pathname === '/reveal') {
      return json(shell.reveal());
    }

    if (request.method === 'POST' && url.pathname === '/answer') {
      const payload = await readAnswerRequest(request);
      if (payload instanceof AnswerPayloadError) {
        return json({ error: payload.message }, 400);
      }
      return json(await shell.submitAnswer(payload.answer, payload.responseTimeMs));
    }

    if (request.method === 'POST' && url.pathname === '/next') {
      return json(await shell.next());
    }

    return new Response('Not found', { status: 404 });
  },
});

console.log(`Web shell listening on http://${hostname}:${server.port}`);
setInterval(() => {}, 60_000);

function json(value: unknown, status = 200): Response {
  return Response.json(value, {
    status,
    headers: { 'cache-control': 'no-store' },
  });
}

async function readAnswerRequest(request: Request): Promise<AnswerPayload | AnswerPayloadError> {
  try {
    return readAnswerPayload(await request.json());
  } catch (error) {
    const message = error instanceof Error ? error.message : 'Invalid answer payload';
    return new AnswerPayloadError(message);
  }
}

function readAnswerPayload(value: unknown): AnswerPayload {
  if (typeof value !== 'object' || value === null) {
    throw new Error('Answer payload must be an object');
  }

  const record = value as Record<string, unknown>;
  const answer = record.answer;
  const responseTimeMs = record.responseTimeMs;

  if (typeof answer !== 'string') {
    throw new Error('Answer payload requires a string answer');
  }

  if (
    typeof responseTimeMs !== 'number' ||
    !Number.isFinite(responseTimeMs) ||
    responseTimeMs < 0
  ) {
    throw new Error('Answer payload requires a non-negative numeric responseTimeMs');
  }

  return { answer, responseTimeMs };
}

class AnswerPayloadError extends Error {}
