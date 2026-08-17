/**
 * Scry CI pipeline.
 *
 * One canonical place to run browser behavior contracts, Rust formatting,
 * tests, linting, documentation, and secret scanning. Each function mounts
 * the source and runs the corresponding command. The `check` function runs
 * all gates in sequence and is what CI and agents should invoke before a merge.
 */
import {
  argument,
  CacheSharingMode,
  type Container,
  type Directory,
  type Service,
  dag,
  func,
  object,
} from '@dagger.io/dagger';

const BUN_IMAGE = 'oven/bun:1.3.14';
const GITLEAKS_IMAGE = 'zricethezav/gitleaks:v8.30.0';
const POSTGRES_IMAGE = 'postgres:17-alpine';
const POSTGRES_TEST_URL = 'postgres://postgres:postgres@postgres:5432/postgres?sslmode=disable';
const RUST_IMAGE = 'rust:1.94-bookworm';
const SOURCE_EXCLUDES = ['.git/', '.tmp/', 'target/'];

function ciSource(source: Directory): Directory {
  return source.filter({ gitignore: true, exclude: SOURCE_EXCLUDES });
}

function rustContainer(source: Directory): Container {
  return dag
    .container()
    .from(RUST_IMAGE)
    .withMountedDirectory('/src', ciSource(source))
    .withMountedCache('/usr/local/cargo/registry', dag.cacheVolume('memory-engine-cargo-registry'))
    .withMountedCache('/usr/local/cargo/git', dag.cacheVolume('memory-engine-cargo-git'))
    .withMountedCache('/cargo-target', dag.cacheVolume('memory-engine-cargo-target'), {
      sharing: CacheSharingMode.Locked,
    })
    .withEnvVariable('CARGO_BUILD_JOBS', '2')
    .withEnvVariable('CARGO_TARGET_DIR', '/cargo-target')
    .withWorkdir('/src')
    .withExec(['rustup', 'component', 'add', 'rustfmt', 'clippy']);
}

function postgresService(): Service {
  return dag
    .container()
    .from(POSTGRES_IMAGE)
    .withEnvVariable('POSTGRES_USER', 'postgres')
    .withEnvVariable('POSTGRES_PASSWORD', 'postgres')
    .withEnvVariable('POSTGRES_DB', 'postgres')
    .withExposedPort(5432)
    .asService();
}

@object()
export class MemoryEngine {
  /**
   * Base Rust container with formatting and lint components available.
   */
  @func()
  rustBase(@argument({ ignore: SOURCE_EXCLUDES }) source: Directory): Container {
    return rustContainer(source);
  }

  /**
   * Run the static browser lifecycle behavior contract.
   */
  @func()
  async browserContract(
    @argument({ ignore: SOURCE_EXCLUDES }) source: Directory,
  ): Promise<string> {
    return dag
      .container()
      .from(BUN_IMAGE)
      .withMountedDirectory('/src', ciSource(source))
      .withWorkdir('/src')
      .withExec(['bun', 'test', 'crates/memory-engine-api/tests/app_js_contract.test.js'])
      .stdout();
  }

  /**
   * Run `cargo fmt --all --check`.
   */
  @func()
  async rustFmt(@argument({ ignore: SOURCE_EXCLUDES }) source: Directory): Promise<string> {
    return this.rustBase(source).withExec(['cargo', 'fmt', '--all', '--check']).stdout();
  }

  /**
   * Run `cargo test --workspace`.
   */
  @func()
  async rustTest(@argument({ ignore: SOURCE_EXCLUDES }) source: Directory): Promise<string> {
    return this.rustBase(source)
      .withServiceBinding('postgres', postgresService())
      .withEnvVariable('MEMORY_ENGINE_POSTGRES_TEST_URL', POSTGRES_TEST_URL)
      .withExec([
        'bash',
        '-c',
        'for attempt in $(seq 1 20); do (echo >/dev/tcp/postgres/5432) >/dev/null 2>&1 && break; sleep 1; done; cargo test --workspace',
      ])
      .stdout();
  }
  /**
   * Run the in-process browser action latency suite against the CI Postgres service.
   * The receipt and markdown report are captured under the known
   * `/tmp/memory-engine-perf` artifact directory.
   */
  @func()
  async actionLatencyPostgres(
    @argument({ ignore: SOURCE_EXCLUDES }) source: Directory,
    gitSha: string,
  ): Promise<Directory> {
    return rustContainer(source)
      .withServiceBinding('postgres', postgresService())
      .withEnvVariable('MEMORY_ENGINE_POSTGRES_TEST_URL', POSTGRES_TEST_URL)
      .withEnvVariable('MEMORY_ENGINE_PERF_GIT_SHA', gitSha)
      .withExec([
        'bash',
        '-c',
        [
          'for attempt in $(seq 1 20); do (echo >/dev/tcp/postgres/5432) >/dev/null 2>&1 && break; sleep 1; done',
          'mkdir -p /tmp/memory-engine-perf',
          'cargo run -p memory-engine-qa -- latency --backend postgres --iterations 3 --out /tmp/memory-engine-perf/action-latency-postgres.json --markdown /tmp/memory-engine-perf/action-latency-postgres.md',
        ].join(' && '),
      ])
      .directory('/tmp/memory-engine-perf');
  }


  /**
   * Run `cargo clippy --workspace --all-targets -- -D warnings`.
   */
  @func()
  async rustClippy(@argument({ ignore: SOURCE_EXCLUDES }) source: Directory): Promise<string> {
    return this.rustBase(source)
      .withExec(['cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'])
      .stdout();
  }

  /**
   * Run `cargo doc --workspace --no-deps`.
   */
  @func()
  async rustDoc(@argument({ ignore: SOURCE_EXCLUDES }) source: Directory): Promise<string> {
    return this.rustBase(source).withExec(['cargo', 'doc', '--workspace', '--no-deps']).stdout();
  }

  /**
   * Scan the mounted source tree for hard-coded secrets with Gitleaks.
   */
  @func()
  async secrets(@argument({ ignore: SOURCE_EXCLUDES }) source: Directory): Promise<string> {
    return dag
      .container()
      .from(GITLEAKS_IMAGE)
      .withMountedDirectory('/src', ciSource(source))
      .withWorkdir('/src')
      .withExec(['gitleaks', 'dir', '/src', '--redact', '--no-banner'])
      .stdout();
  }

  /**
   * Run every gate in sequence. A non-zero exit on any gate fails the pipeline.
   * Returns a concatenated log on success.
   */
  @func()
  async check(
    @argument({ ignore: SOURCE_EXCLUDES }) source: Directory,
    gitSha: string,
  ): Promise<string> {
    const browserContract = await this.browserContract(source);
    const rustFmt = await this.rustFmt(source);
    const rustTest = await this.rustTest(source);
    const actionLatencyArtifact = await this.actionLatencyPostgres(source, gitSha);
    const actionLatencyReceipt = await actionLatencyArtifact
      .file('action-latency-postgres.json')
      .contents();
    const actionLatencyMarkdown = await actionLatencyArtifact
      .file('action-latency-postgres.md')
      .contents();
    const rustClippy = await this.rustClippy(source);
    const rustDoc = await this.rustDoc(source);
    const secrets = await this.secrets(source);
    return [
      `=== browser contract ===\n${browserContract}`,
      `=== rust fmt ===\n${rustFmt}`,
      `=== rust test ===\n${rustTest}`,
      `=== action latency (postgres) ===\n${actionLatencyReceipt}\n${actionLatencyMarkdown}`,
      `=== rust clippy ===\n${rustClippy}`,
      `=== rust doc ===\n${rustDoc}`,
      `=== secrets ===\n${secrets}`,
    ].join('\n');
  }
}
