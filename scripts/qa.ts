type QaMode = 'full' | 'local';

type QaLane = {
  id: string;
  title: string;
  surface: string;
  purpose: string;
  command: readonly string[];
  gating: boolean;
  modes: readonly QaMode[];
};

type LaneReceipt = {
  lane: QaLane;
  exitCode: number;
  elapsedMs: number;
};

const lanes: readonly QaLane[] = [
  {
    id: 'static.typecheck',
    title: 'TypeScript strict contract',
    surface: 'all package code included by tsconfig',
    purpose: 'Catch type drift across public contracts and behavior tests.',
    command: ['bun', 'run', 'typecheck'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'static.biome',
    title: 'Biome lint and format',
    surface: 'repo source, tests, scripts, docs-adjacent code',
    purpose: 'Keep strict style, unused imports, and unsafe patterns out of QA evidence.',
    command: ['bun', 'run', 'check'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'api.exports',
    title: 'Public package exports',
    surface: 'memory-engine, subpath exports, root compatibility',
    purpose: 'Prove consumers can compose API surfaces without private src imports.',
    command: ['bun', 'test', 'tests/api/module-exports.test.ts', 'tests/api/compatibility.test.ts'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'kernel.types-scheduler',
    title: 'Types and scheduler behavior',
    surface: 'ReviewUnitId, ScheduleState, FSRS next-state transition',
    purpose: 'Protect JSON-safe schedule state and ts-fsrs round-trip semantics.',
    command: [
      'bun',
      'test',
      'tests/types/',
      'tests/scheduler/roundtrip.test.ts',
      'tests/scheduler/serialize.test.ts',
    ],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'kernel.grader',
    title: 'Deterministic and rubric grading',
    surface: 'Grader, AsyncGrader, rating policy, prompt exhaustiveness',
    purpose: 'Protect one-envelope grade results and fixed verdict semantics.',
    command: ['bun', 'test', 'tests/grader/'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'kernel.progression-queue',
    title: 'Progression and queue behavior',
    surface: 'mastery, prerequisites, supersession, due ordering, anti-clumping',
    purpose: 'Prove the actual learning flow selects eligible work in stable order.',
    command: ['bun', 'test', 'tests/progression/', 'tests/queue/'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'contracts.testkit-adapters',
    title: 'Testkit and adapter contracts',
    surface: 'memory-engine/testkit, memory-engine/adapters',
    purpose: 'Prove shared fixtures and adapter doubles remain usable consumer contracts.',
    command: ['bun', 'test', 'tests/testkit/', 'tests/adapters/'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'service.prototype',
    title: 'Service prototype behavior',
    surface: 'repo-local service command boundary',
    purpose: 'Prove injected persistence, command flow, and failure semantics stay explicit.',
    command: ['bun', 'test', 'tests/service/'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'evals.regression-corpus',
    title: 'Learning behavior regression corpus',
    surface: 'fixtures replayed through live public API surfaces',
    purpose: 'Catch semantic drift across grading, scheduling, progression, and queue behavior.',
    command: ['bun', 'test', 'tests/evals/regression-corpus.test.ts'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'dogfood.experiments',
    title: 'Dogfood client receipts',
    surface: 'CLI review, import probe, web shell',
    purpose: 'Exercise API ergonomics from clients outside src and record boundary pressure.',
    command: [
      'bun',
      'test',
      'experiments/cli-review/cli-review.test.ts',
      'experiments/import-probe/import-probe.test.ts',
      'experiments/web-shell/web-shell.test.ts',
    ],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'coverage.all',
    title: 'Coverage-enforced full test sweep',
    surface: 'all Bun tests included by the repo',
    purpose: 'Preserve broad executable confidence and coverage floor evidence.',
    command: ['bun', 'run', 'coverage'],
    gating: true,
    modes: ['local', 'full'],
  },
  {
    id: 'performance.benchmarks',
    title: 'Non-gating benchmark receipts',
    surface: 'grader, scheduler, queue, service composition',
    purpose: 'Expose performance-quality drift without brittle local thresholds.',
    command: ['bun', 'run', 'bench'],
    gating: false,
    modes: ['local', 'full'],
  },
  {
    id: 'ci.canonical',
    title: 'Canonical Dagger CI gate',
    surface: 'install, typecheck, Biome, coverage, Gitleaks',
    purpose: 'Prove handoff quality with the repository gate, not adjacent evidence.',
    command: ['bun', 'run', 'ci'],
    gating: true,
    modes: ['full'],
  },
];

async function main(): Promise<void> {
  const mode = parseMode(Bun.argv.slice(2));
  const selected = lanes.filter((lane) => lane.modes.includes(mode));
  const startedAt = Date.now();
  const receipts: LaneReceipt[] = [];
  let failed = false;

  printHeader(mode, selected);

  for (const lane of selected) {
    const receipt = await runLane(lane);
    receipts.push(receipt);
    printReceipt(receipt);

    if (receipt.exitCode !== 0 && lane.gating) {
      failed = true;
      break;
    }
  }

  printSummary(mode, receipts, Date.now() - startedAt);

  if (failed) {
    process.exit(1);
  }
}

function parseMode(args: readonly string[]): QaMode {
  if (args.includes('--local')) {
    return 'local';
  }

  if (args.includes('--full')) {
    return 'full';
  }

  return 'full';
}

function printHeader(mode: QaMode, selected: readonly QaLane[]): void {
  console.log(`# memory-engine QA (${mode})`);
  console.log('');
  console.log(`lanes: ${selected.length}`);
  console.log('');
}

async function runLane(lane: QaLane): Promise<LaneReceipt> {
  console.log(`## ${lane.id}: ${lane.title}`);
  console.log(`surface: ${lane.surface}`);
  console.log(`purpose: ${lane.purpose}`);
  console.log(`command: ${shellCommand(lane.command)}`);
  console.log('');

  const start = performance.now();
  const child = Bun.spawn([...lane.command], {
    cwd: process.cwd(),
    stdout: 'inherit',
    stderr: 'inherit',
  });
  const exitCode = await child.exited;

  return {
    lane,
    exitCode,
    elapsedMs: performance.now() - start,
  };
}

function printReceipt(receipt: LaneReceipt): void {
  const status = receipt.exitCode === 0 ? 'PASS' : receipt.lane.gating ? 'FAIL' : 'WARN';

  console.log('');
  console.log(
    `receipt: ${status} ${receipt.lane.id} exit=${receipt.exitCode} elapsed_ms=${receipt.elapsedMs.toFixed(0)}`,
  );
  console.log('');
}

function printSummary(mode: QaMode, receipts: readonly LaneReceipt[], elapsedMs: number): void {
  const failed = receipts.filter((receipt) => receipt.exitCode !== 0 && receipt.lane.gating);
  const warned = receipts.filter((receipt) => receipt.exitCode !== 0 && !receipt.lane.gating);

  console.log('# QA summary');
  console.log(`mode: ${mode}`);
  console.log(`elapsed_ms: ${elapsedMs}`);
  console.log(`passed_lanes: ${receipts.filter((receipt) => receipt.exitCode === 0).length}`);
  console.log(`warning_lanes: ${warned.length}`);
  console.log(`failed_lanes: ${failed.length}`);

  if (failed.length > 0) {
    console.log('');
    console.log('failed:');
    for (const receipt of failed) {
      console.log(`- ${receipt.lane.id}: ${shellCommand(receipt.lane.command)}`);
    }
  }
}

function shellCommand(command: readonly string[]): string {
  return command.map(quoteShellArg).join(' ');
}

function quoteShellArg(value: string): string {
  if (/^[a-zA-Z0-9_./:@=-]+$/.test(value)) {
    return value;
  }

  return `'${value.replace(/'/g, "'\\''")}'`;
}

await main();

export {};
