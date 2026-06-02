import { describe, expect, test } from 'bun:test';

import {
  buildBetaStudyProcess,
  chooseShareUrl,
  parseArgs,
  parseIpv4Addresses,
  serverStatePath,
} from './local-server';

describe('local server manager helpers', () => {
  test('prefers a Tailscale address for share URLs when available', () => {
    const url = chooseShareUrl({
      port: 4177,
      preferredHost: '0.0.0.0',
      ipv4Addresses: ['127.0.0.1', '10.1.10.156', '100.98.47.112'],
    });

    expect(url).toBe('http://100.98.47.112:4177/');
  });

  test('falls back to localhost when no shareable address is present', () => {
    const url = chooseShareUrl({
      port: 4177,
      preferredHost: '127.0.0.1',
      ipv4Addresses: ['127.0.0.1'],
    });

    expect(url).toBe('http://127.0.0.1:4177/');
  });

  test('parses IPv4 addresses from ifconfig-style output', () => {
    expect(
      parseIpv4Addresses(`
        inet 127.0.0.1 netmask 0xff000000
        inet 10.1.10.156 netmask 0xffffff00 broadcast 10.1.10.255
        inet 100.98.47.112 --> 100.98.47.112 netmask 0xffffffff
      `),
    ).toEqual(['127.0.0.1', '10.1.10.156', '100.98.47.112']);
  });

  test('builds a beta-study server process with durable store and log paths', () => {
    const process = buildBetaStudyProcess({
      cwd: '/repo',
      host: '0.0.0.0',
      port: 4177,
      store: '.tmp/beta-study/store.json',
    });

    expect(process.command).toBe('bun');
    expect(process.args).toEqual(['run', 'experiments/beta-study/server.ts']);
    expect(process.env).toMatchObject({
      HOST: '0.0.0.0',
      PORT: '4177',
      BETA_STUDY_STORE: '.tmp/beta-study/store.json',
    });
    expect(process.logPath).toBe('/repo/.tmp/local-servers/beta-study.log');
  });

  test('parses command defaults without relying on remembered ports', () => {
    expect(parseArgs(['start'])).toMatchObject({
      command: 'start',
      name: 'beta-study',
      host: '0.0.0.0',
      port: 4177,
      store: '.tmp/beta-study/store.json',
      reset: false,
    });
  });

  test('keeps manager state under the repo tmp directory', () => {
    expect(serverStatePath('/repo', 'beta-study')).toBe('/repo/.tmp/local-servers/beta-study.json');
  });
});
