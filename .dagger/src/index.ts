/**
 * Memory Engine CI pipeline.
 *
 * One canonical place to run typecheck, lint, coverage-enforced tests,
 * and secret scanning against a Bun workspace. Each function mounts the
 * source, installs dependencies inside a Bun container, and runs the
 * corresponding package script. The `check` function runs all gates in
 * sequence and is what CI and agents should invoke before proposing a merge.
 */
import { type Container, type Directory, dag, func, object } from '@dagger.io/dagger';

const BUN_IMAGE = 'oven/bun:1.3.9-alpine';
const GITLEAKS_IMAGE = 'zricethezav/gitleaks:v8.30.0';

@object()
export class MemoryEngine {
  /**
   * Base container with source mounted and dependencies installed.
   */
  @func()
  base(source: Directory): Container {
    return dag
      .container()
      .from(BUN_IMAGE)
      .withMountedDirectory('/src', source)
      .withWorkdir('/src')
      .withExec(['bun', 'install', '--frozen-lockfile']);
  }

  /**
   * Run `bun run typecheck`.
   */
  @func()
  async typecheck(source: Directory): Promise<string> {
    return this.base(source).withExec(['bun', 'run', 'typecheck']).stdout();
  }

  /**
   * Run `bun run check` (Biome lint + format).
   */
  @func()
  async lint(source: Directory): Promise<string> {
    return this.base(source).withExec(['bun', 'run', 'check']).stdout();
  }

  /**
   * Run `bun test`.
   */
  @func()
  async test(source: Directory): Promise<string> {
    return this.base(source).withExec(['bun', 'test']).stdout();
  }

  /**
   * Run `bun run coverage` to enforce the repository coverage floor.
   */
  @func()
  async coverage(source: Directory): Promise<string> {
    return this.base(source).withExec(['bun', 'run', 'coverage']).stdout();
  }

  /**
   * Scan the mounted source tree for hard-coded secrets with Gitleaks.
   */
  @func()
  async secrets(source: Directory): Promise<string> {
    return dag
      .container()
      .from(GITLEAKS_IMAGE)
      .withMountedDirectory('/src', source)
      .withWorkdir('/src')
      .withExec(['gitleaks', 'dir', '/src', '--redact', '--no-banner'])
      .stdout();
  }

  /**
   * Run every gate in sequence: typecheck → lint/format →
   * coverage-enforced tests → secrets scan. A non-zero exit on any
   * gate fails the pipeline. Returns a concatenated log on success.
   */
  @func()
  async check(source: Directory): Promise<string> {
    const base = this.base(source);
    const typecheck = await base.withExec(['bun', 'run', 'typecheck']).stdout();
    const lint = await base.withExec(['bun', 'run', 'check']).stdout();
    const coverage = await base.withExec(['bun', 'run', 'coverage']).stdout();
    const secrets = await this.secrets(source);
    return [
      `=== typecheck ===\n${typecheck}`,
      `=== lint ===\n${lint}`,
      `=== coverage ===\n${coverage}`,
      `=== secrets ===\n${secrets}`,
    ].join('\n');
  }
}
