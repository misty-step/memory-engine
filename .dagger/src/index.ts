/**
 * Memory Engine CI pipeline.
 *
 * One canonical place to run typecheck, lint, format-check, and tests
 * against a Bun workspace. Each function mounts the source, installs
 * dependencies inside a Bun container, and runs the corresponding
 * package script. The `ci` function runs all gates in sequence and is
 * what CI and agents should invoke before proposing a merge.
 */
import { type Container, type Directory, dag, func, object } from '@dagger.io/dagger';

const BUN_IMAGE = 'oven/bun:1.3.9-alpine';

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
  async check(source: Directory): Promise<string> {
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
   * Run every gate in sequence: typecheck → lint/format → tests.
   * A non-zero exit on any gate fails the pipeline. Returns a
   * concatenated log on success.
   */
  @func()
  async ci(source: Directory): Promise<string> {
    const base = this.base(source);
    const typecheck = await base.withExec(['bun', 'run', 'typecheck']).stdout();
    const check = await base.withExec(['bun', 'run', 'check']).stdout();
    const test = await base.withExec(['bun', 'test']).stdout();
    return `=== typecheck ===\n${typecheck}\n=== check ===\n${check}\n=== test ===\n${test}`;
  }
}
