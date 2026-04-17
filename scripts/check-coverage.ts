const COVERAGE_FLOOR = 80;

const processHandle = Bun.spawn(['bun', 'test', '--coverage'], {
  cwd: process.cwd(),
  stdout: 'pipe',
  stderr: 'pipe',
});

const [stdout, stderr, exitCode] = await Promise.all([
  new Response(processHandle.stdout).text(),
  new Response(processHandle.stderr).text(),
  processHandle.exited,
]);

const combinedOutput = `${stdout}\n${stderr}`;

process.stdout.write(stdout);
process.stderr.write(stderr);

if (exitCode !== 0) {
  process.exit(exitCode);
}

const summaryMatch = combinedOutput.match(/All files\s+\|\s+([\d.]+)\s+\|\s+([\d.]+)/);
if (!summaryMatch) {
  console.error('Coverage summary not found in bun test --coverage output.');
  process.exit(1);
}

const functionCoverage = Number(summaryMatch[1]);
const lineCoverage = Number(summaryMatch[2]);

if (!Number.isFinite(functionCoverage) || !Number.isFinite(lineCoverage)) {
  console.error('Coverage summary contained non-numeric values.');
  process.exit(1);
}

if (functionCoverage < COVERAGE_FLOOR || lineCoverage < COVERAGE_FLOOR) {
  console.error(
    `Coverage floor ${COVERAGE_FLOOR}% not met (funcs=${functionCoverage.toFixed(2)}%, lines=${lineCoverage.toFixed(2)}%).`,
  );
  process.exit(1);
}

console.log(
  `Coverage floor ${COVERAGE_FLOOR}% met (funcs=${functionCoverage.toFixed(2)}%, lines=${lineCoverage.toFixed(2)}%).`,
);
