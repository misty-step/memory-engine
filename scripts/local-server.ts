import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';

type Command = 'start' | 'stop' | 'status' | 'list' | 'open' | 'url';
type ServerName = 'beta-study';

type ParsedArgs = {
  command: Command;
  name: ServerName;
  host: string;
  port: number;
  store: string;
  reset: boolean;
};

type ProcessSpec = {
  command: string;
  args: string[];
  env: Record<string, string>;
  logPath: string;
};

type ServerState = {
  name: ServerName;
  pid: number;
  host: string;
  port: number;
  localUrl: string;
  shareUrl: string;
  store: string;
  logPath: string;
  startedAt: string;
};

const defaultServerName = 'beta-study' satisfies ServerName;
const defaultHost = '0.0.0.0';
const defaultPort = 4177;
const defaultStore = '.tmp/beta-study/store.json';

export function parseArgs(argv: string[]): ParsedArgs {
  const [rawCommand, ...rest] = argv;
  const command = parseCommand(rawCommand ?? 'status');
  const options = new Map<string, string | true>();
  let name: ServerName = defaultServerName;

  for (let index = 0; index < rest.length; index += 1) {
    const token = rest[index];
    if (token === undefined) {
      continue;
    }
    if (!token.startsWith('--')) {
      name = parseServerName(token);
      continue;
    }

    const key = token.slice(2);
    if (key === 'reset') {
      options.set(key, true);
      continue;
    }

    const value = rest[index + 1];
    if (value === undefined || value.startsWith('--')) {
      throw new Error(`Missing value for --${key}`);
    }
    options.set(key, value);
    index += 1;
  }

  return {
    command,
    name,
    host: stringOption(options, 'host', defaultHost),
    port: numberOption(options, 'port', defaultPort),
    store: stringOption(options, 'store', defaultStore),
    reset: options.get('reset') === true,
  };
}

export function serverStatePath(cwd: string, name: ServerName): string {
  return join(cwd, '.tmp', 'local-servers', `${name}.json`);
}

export function buildBetaStudyProcess(input: {
  cwd: string;
  host: string;
  port: number;
  store: string;
}): ProcessSpec {
  return {
    command: 'bun',
    args: ['run', 'experiments/beta-study/server.ts'],
    env: {
      HOST: input.host,
      PORT: input.port.toString(),
      BETA_STUDY_STORE: input.store,
    },
    logPath: join(input.cwd, '.tmp', 'local-servers', 'beta-study.log'),
  };
}

export function parseIpv4Addresses(output: string): string[] {
  return Array.from(
    output.matchAll(/\binet\s+(\d+\.\d+\.\d+\.\d+)\b/g),
    (match) => match[1],
  ).filter((address): address is string => address !== undefined);
}

export function chooseShareUrl(input: {
  port: number;
  preferredHost: string;
  ipv4Addresses: string[];
}): string {
  const tailscale = input.ipv4Addresses.find((address) => address.startsWith('100.'));
  if (tailscale !== undefined) {
    return `http://${tailscale}:${input.port}/`;
  }

  const lan = input.ipv4Addresses.find(
    (address) => !address.startsWith('127.') && !address.startsWith('169.254.'),
  );
  if (lan !== undefined && input.preferredHost !== '127.0.0.1') {
    return `http://${lan}:${input.port}/`;
  }

  return `http://127.0.0.1:${input.port}/`;
}

async function main(): Promise<void> {
  const cwd = process.cwd();
  const args = parseArgs(Bun.argv.slice(2));

  switch (args.command) {
    case 'start':
      await startServer(cwd, args);
      break;
    case 'stop':
      await stopServer(cwd, args.name);
      break;
    case 'status':
      await printStatus(cwd, args.name);
      break;
    case 'list':
      await printStatus(cwd, args.name);
      break;
    case 'open':
      await openServer(cwd, args.name);
      break;
    case 'url':
      await printUrl(cwd, args.name);
      break;
  }
}

async function startServer(cwd: string, args: ParsedArgs): Promise<void> {
  await mkdir(join(cwd, '.tmp', 'local-servers'), { recursive: true });

  const existing = await readState(cwd, args.name);
  if (existing !== null && isProcessRunning(existing.pid)) {
    printState('already-running', existing);
    return;
  }

  if (args.reset) {
    await rm(join(cwd, args.store), { force: true });
  }

  await mkdir(dirname(join(cwd, args.store)), { recursive: true });
  const spec = buildBetaStudyProcess({
    cwd,
    host: args.host,
    port: args.port,
    store: args.store,
  });
  const shellCommand = [
    shellQuote(spec.command),
    ...spec.args.map(shellQuote),
    '>>',
    shellQuote(spec.logPath),
    '2>&1',
  ].join(' ');
  const child = spawn('/bin/zsh', ['-lc', `exec ${shellCommand}`], {
    cwd,
    env: { ...process.env, ...spec.env },
    detached: true,
    stdio: 'ignore',
  });
  child.unref();
  if (child.pid === undefined) {
    throw new Error(`Failed to start ${args.name}`);
  }

  const localUrl = `http://127.0.0.1:${args.port}/`;
  const shareUrl = chooseShareUrl({
    port: args.port,
    preferredHost: args.host,
    ipv4Addresses: await localIpv4Addresses(),
  });
  const state: ServerState = {
    name: args.name,
    pid: child.pid,
    host: args.host,
    port: args.port,
    localUrl,
    shareUrl,
    store: args.store,
    logPath: spec.logPath,
    startedAt: new Date().toISOString(),
  };

  await writeState(cwd, state);
  await waitForHttp(localUrl);
  printState('started', state);
}

async function stopServer(cwd: string, name: ServerName): Promise<void> {
  const state = await readState(cwd, name);
  if (state === null) {
    console.log(`${name}: stopped`);
    return;
  }

  if (isProcessRunning(state.pid)) {
    process.kill(state.pid, 'SIGTERM');
  }
  await rm(serverStatePath(cwd, name), { force: true });
  console.log(`${name}: stopped`);
}

async function printStatus(cwd: string, name: ServerName): Promise<void> {
  const state = await readState(cwd, name);
  if (state === null || !isProcessRunning(state.pid)) {
    if (state !== null) {
      await rm(serverStatePath(cwd, name), { force: true });
    }
    console.log(`${name}: stopped`);
    return;
  }

  printState('running', state);
}

async function printUrl(cwd: string, name: ServerName): Promise<void> {
  const state = await requireRunningState(cwd, name);
  console.log(state.shareUrl);
}

async function openServer(cwd: string, name: ServerName): Promise<void> {
  const state = await requireRunningState(cwd, name);
  Bun.spawn(['open', '-a', 'Google Chrome', state.shareUrl]).unref();
  printState('opened', state);
}

async function requireRunningState(cwd: string, name: ServerName): Promise<ServerState> {
  const state = await readState(cwd, name);
  if (state === null || !isProcessRunning(state.pid)) {
    throw new Error(`${name} is not running. Start it with: bun run local:server start`);
  }
  return state;
}

async function readState(cwd: string, name: ServerName): Promise<ServerState | null> {
  const path = serverStatePath(cwd, name);
  if (!existsSync(path)) {
    return null;
  }

  return JSON.parse(await readFile(path, 'utf8')) as ServerState;
}

async function writeState(cwd: string, state: ServerState): Promise<void> {
  const path = serverStatePath(cwd, state.name);
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(state, null, 2)}\n`);
}

function isProcessRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function localIpv4Addresses(): Promise<string[]> {
  const command = Bun.spawn(['ifconfig'], { stdout: 'pipe', stderr: 'ignore' });
  const output = await new Response(command.stdout).text();
  await command.exited;
  return parseIpv4Addresses(output);
}

async function waitForHttp(url: string): Promise<void> {
  const deadline = Date.now() + 5_000;
  let lastError: unknown = null;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
      lastError = new Error(`${url} returned ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await Bun.sleep(100);
  }

  throw new Error(`Server did not answer ${url}: ${String(lastError)}`);
}

function printState(label: string, state: ServerState): void {
  console.log(`${state.name}: ${label}`);
  console.log(`  pid: ${state.pid}`);
  console.log(`  local: ${state.localUrl}`);
  console.log(`  share: ${state.shareUrl}`);
  console.log(`  store: ${state.store}`);
  console.log(`  log: ${state.logPath}`);
}

function parseCommand(value: string): Command {
  if (
    value === 'start' ||
    value === 'stop' ||
    value === 'status' ||
    value === 'list' ||
    value === 'open' ||
    value === 'url'
  ) {
    return value;
  }
  throw new Error(`Unknown local server command: ${value}`);
}

function parseServerName(value: string): ServerName {
  if (value === 'beta-study') {
    return value;
  }
  throw new Error(`Unknown local server: ${value}`);
}

function stringOption(options: Map<string, string | true>, key: string, fallback: string): string {
  const value = options.get(key);
  if (typeof value === 'string') {
    return value;
  }
  return fallback;
}

function numberOption(options: Map<string, string | true>, key: string, fallback: number): number {
  const value = options.get(key);
  if (typeof value !== 'string') {
    return fallback;
  }

  const parsed = Number.parseInt(value, 10);
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    throw new Error(`--${key} must be a positive integer`);
  }
  return parsed;
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

if (basename(Bun.argv[1] ?? '') === 'local-server.ts') {
  main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
