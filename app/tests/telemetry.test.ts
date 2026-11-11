import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { TransportItem } from '@grafana/faro-web-sdk';
import { environmentFor, isLocalHost, redactText, sanitizeItem } from '../src/app/telemetry.ts';

const sanitizePayload = (payload: object): Record<string, unknown> => {
  const out = sanitizeItem({ type: 'log', meta: {}, payload } as unknown as TransportItem);
  assert.ok(out);
  return (out as unknown as { payload: Record<string, unknown> }).payload;
};

describe('environmentFor and isLocalHost', () => {
  it('maps dev, interne, home and prod hostnames', () => {
    assert.equal(environmentFor('lfsx.dev.ferrlabs'), 'dev');
    assert.equal(environmentFor('lfsx.ferrlabs'), 'interne');
    assert.equal(environmentFor('nas.home'), 'home');
    assert.equal(environmentFor('lfsx.example.com'), 'prod');
  });

  it('does not match lookalike suffixes', () => {
    assert.equal(environmentFor('evilferrlabs'), 'prod');
  });

  it('recognises local hosts', () => {
    assert.equal(isLocalHost('localhost'), true);
    assert.equal(isLocalHost('lfsx.example.com'), false);
  });
});

describe('redactText leaks', () => {
  const leaks: ReadonlyArray<readonly [string, string]> = [
    ['password=hunter2abc', 'hunter2abc'],
    ['{"password":"hunter2abc"}', 'hunter2abc'],
    ['Authorization: Bearer abcdefgh1234', 'abcdefgh1234'],
    ['Authorization: Basic dXNlcjpwYXNzd29yZAabc==', 'dXNlcjpwYXNzd29yZAabc'],
    [['eyJhbGciOiJIUzI1NiJ9', 'eyJzdWIiOiJhZG1pbiJ9abc', 'c2lnbmF0dXJlabc'].join('.'), 'c2lnbmF0dXJlabc'],
    ['https://alice:hunter2abc@db.example.com/x', 'hunter2abc'],
    ['contact jane.doe@example.com today', 'jane.doe'],
    ['reset?token=abc123xyz', 'abc123xyz'],
    ['user[password]=hunter2abc', 'hunter2abc'],
    ['{"user[password]":"hunter2abc"}', 'hunter2abc'],
    ['user[password]: hunter2abc', 'hunter2abc'],
    ['params[api_key]=x1abc', 'x1abc'],
    ["password='it''s''my''pass''hunter2abc'", 'hunter2abc'],
    ['email_verification_code=834201', '834201'],
    ['user_mfa_code=834201', '834201'],
    ['user_code=WDJB-MJHT', 'WDJB'],
    ['sms_code=abc123', 'abc123'],
    ['backup_code=abc123', 'abc123'],
    ['params[user_code]=abc123', 'abc123'],
    ['token_code=abc123', 'abc123'],
    ['session_state=abc123', 'abc123'],
    ['relay_state=abc123', 'abc123'],
    ['storage_key=abc123', 'abc123'],
    ['daemonKey=abc123', 'abc123'],
    ['commonKey=abc123', 'abc123'],
    ['normalKey=abc123', 'abc123'],
    ['minimalKey=abc123', 'abc123'],
    ['codeSig=abc123', 'abc123'],
    ['nodeSignature=abc123', 'abc123'],
    ['passkey=abc123', 'abc123'],
    ['see //password:aJ@+QvZ7 end', 'QvZ7'],
  ];

  for (const [input, marker] of leaks) {
    it(`masks ${JSON.stringify(input)}`, () => {
      assert.ok(!redactText(input).includes(marker), redactText(input));
    });
  }

  it('is idempotent on auth=""\'\'""\'\'{', () => {
    const once = redactText('auth=""\'\'""\'\'{');
    assert.equal(redactText(once), once);
  });
});

describe('redactText kept text', () => {
  const kept = [
    'error_code=500',
    'status_code=200',
    'errorCode=500',
    'zip_code=59000',
    'barcode=123',
    'keyCode=13',
    'readyState=complete',
    'game_state=lobby',
    'httpStatusCode=200',
    '//auth:8080/x',
    'hotkey: Ctrl',
    'state: loading',
    'time 12:30',
  ];

  for (const input of kept) {
    it(`keeps ${JSON.stringify(input)}`, () => {
      assert.equal(redactText(input), input);
    });
  }

  const benignKeyWords = [
    'dickey', 'donkey', 'doohickey', 'flukey', 'flunkey', 'hickey', 'hockey', 'hokey', 'hookey', 'hotkey',
    'jockey', 'lackey', 'malarkey', 'mickey', 'monkey', 'pokey', 'rickey', 'smokey', 'turkey', 'turnkey', 'whiskey',
  ];
  for (const word of benignKeyWords) {
    it(`keeps ${word}=3`, () => {
      assert.equal(redactText(`${word}=3`), `${word}=3`);
    });
  }

  it('keeps the text around a masked host-less userinfo lookalike', () => {
    assert.equal(redactText('see //password:aJ@+QvZ7 end'), 'see //password:*');
  });

  it('does not treat a bracketed host as userinfo', () => {
    assert.equal(redactText('https://[::1]:8080/users/jane@corp.io'), 'https://[::1]:8080/users/*');
  });
});

describe('sanitizeItem field names', () => {
  it('matches benign field exclusions on the key suffix only', () => {
    const payload = sanitizePayload({
      homeCacheKey: 'a',
      x: { idempotencyKey: 'b' },
      user: { authType: 'sso' },
      daemonKey: 'c',
      commonKey: 'd',
      codeSig: 'e',
      escrowKey: 'f',
    });
    assert.deepEqual(payload, {
      homeCacheKey: 'a',
      x: { idempotencyKey: 'b' },
      user: { authType: 'sso' },
      daemonKey: '*',
      commonKey: '*',
      codeSig: '*',
      escrowKey: '*',
    });
  });

  it('masks plural and prefixed code and state field names', () => {
    const payload = sanitizePayload({
      storageKey: 'abc',
      tokens: ['x'],
      apiKeys: 'k',
      otp_code: 123456,
      mfaCode: 'm',
      session_state: 's',
    });
    assert.deepEqual(payload, {
      storageKey: '*',
      tokens: ['*'],
      apiKeys: '*',
      otp_code: '*',
      mfaCode: '*',
      session_state: '*',
    });
  });

  it('keeps bare and benign code and state field names', () => {
    const input = {
      code: 'a',
      state: 'b',
      errorCode: 'c',
      httpStatusCode: 'd',
      readyState: 'e',
      homeCacheKeys: 'f',
      objectKey: 'g',
      idempotencyKey: 'h',
    };
    assert.deepEqual(sanitizePayload(input), input);
  });

  it('keeps every key that redacts to the same string', () => {
    assert.deepEqual(sanitizePayload({ 'a@b.com': 1, 'c@d.com': 2 }), { '*': 1, '*#2': 2 });
  });

  it('masks arrays under a secret field and still redacts the strings inside nested objects', () => {
    assert.deepEqual(
      sanitizePayload({
        token: ['abc', ['nested']],
        apiKey: { note: 'x', url: 'https://a.com/x?code=S3CR3T' },
        list: ['ok'],
      }),
      { token: ['*', ['*']], apiKey: { note: '*', url: '*' }, list: ['ok'] },
    );
  });

  it('masks a container shared between a benign and a secret field', () => {
    const shared = ['hunter2'];
    assert.deepEqual(sanitizePayload({ list: shared, password: shared })['password'], ['*']);
  });
});

describe('redactText bounds', () => {
  const adversarial: ReadonlyArray<readonly [string, string]> = [
    ['spaces after a token word', 'token: a' + ' '.repeat(16000) + 'x'],
    ['spaces after a bare colon', 'token:' + ' '.repeat(16000)],
    ['spaces inside a password value', 'password=ab' + ' '.repeat(16000) + 'x'],
    ['spaces after an auth scheme', 'Authorization: Token' + ' '.repeat(16000) + 'x'],
    ['alternating tabs and spaces', 'token: a' + '\t \t'.repeat(5000) + 'x'],
    ['a newline then spaces', 'token:\n' + ' '.repeat(16000) + 'x'],
    ['query separators', '?a='.repeat(5400)],
    ['repeated token pairs', 'token=abc '.repeat(1600)],
    ['dots', 'a.'.repeat(8000)],
    ['plain text', 'x'.repeat(16000)],
  ];

  for (const [label, input] of adversarial) {
    it(`stays under 250 ms on 16 KB of ${label}`, () => {
      const started = performance.now();
      redactText(input);
      assert.ok(performance.now() - started < 250);
    });
  }
});

describe('secret subtrees, metric names and key normalization', () => {
  const SECRET = 'hunter2abc';
  const subtreeLeaks: ReadonlyArray<readonly [object, string]> = [
    [{ password: { value: SECRET } }, SECRET],
    [{ token: [{ value: SECRET }] }, SECRET],
    [{ apiKey: { primary: SECRET } }, SECRET],
    [{ client_secret: { current: { raw: SECRET } } }, SECRET],
    [{ password: [[SECRET]] }, SECRET],
    [{ secret: { n: 48291234 } }, '48291234'],
    [{ token: { x: 3 } }, '3'],
    [{ auth_count: { x: 3 } }, '3'],
  ];

  for (const [payload, marker] of subtreeLeaks) {
    it(`masks every leaf of ${JSON.stringify(payload)}`, () => {
      const out = JSON.stringify(sanitizePayload(structuredClone(payload)));
      assert.ok(!out.includes(marker), out);
    });
  }

  it('keeps shape, booleans and null under a secret field and leaves other fields alone', () => {
    assert.deepEqual(sanitizePayload({ password: { enabled: true, value: null, deep: { ok: false, v: 'x' } } }), {
      password: { enabled: true, value: null, deep: { ok: false, v: '*' } },
    });
    assert.deepEqual(sanitizePayload({ settings: { theme: 'dark' } }), { settings: { theme: 'dark' } });
  });

  it('keeps numbers under metric names in any case and masks other secret numbers', () => {
    assert.deepEqual(sanitizePayload({ AUTH_COUNT: 3, auth_count: 3, authCount: 3, SESSION_DURATION_MS: 7, TOKEN_TTL: 9 }), {
      AUTH_COUNT: 3,
      auth_count: 3,
      authCount: 3,
      SESSION_DURATION_MS: 7,
      TOKEN_TTL: 9,
    });
    assert.deepEqual(sanitizePayload({ AUTH_TOKEN: 3 }), { AUTH_TOKEN: '*' });
  });

  it('keeps colliding redacted keys under a secret field', () => {
    assert.deepEqual(sanitizePayload({ password: { 'a@b.com': 'x', 'c@d.com': 'y' } }), { password: { '*': '*', '*#2': '*' } });
  });

  it('does not treat separator-joined benign key names as secret', () => {
    for (const key of ['i18n:key', 'cache:key', 'cache/key'])
      assert.deepEqual(Object.values(sanitizePayload({ [key]: 'v' })), ['v'], key);
  });

  it('treats bracketed, slashed, colon-joined and dotted secret names as secret', () => {
    for (const key of ['user[password]', 'headers/authorization', 'a:b:token', 'x.password'])
      assert.deepEqual(Object.values(sanitizePayload({ [key]: 'v' })), ['*'], key);
  });

  it('masks the value of a bare //host:port that is not a real host with a path', () => {
    for (const input of ['//password:12345', 'see //password:12345', '//password:8080'])
      assert.ok(!redactText(input).includes('12345') && !redactText(input).includes('8080'), input);
    assert.equal(redactText('//auth:8080/x'), '//auth:8080/x');
  });
});
describe('redactText v6.37 code and state keys', () => {
  const masked: ReadonlyArray<readonly [string, string]> = [
    ['otp_code: 834201 sent', 'otp_code: *'],
    ['{"mfa_code": "834201"}', '{"mfa_code": *}'],
    ['user_code: WDJB-MJHT', 'user_code: *'],
    ['relay_state: abc123xyz', 'relay_state: *'],
  ];
  const kept: readonly string[] = [
    'Error code: 404',
    'status code: 404',
    'status_code: 200',
    'code: ERR_NETWORK',
    'state: open',
    'readyState: complete',
    'game_state: lobby',
  ];

  for (const [input, expected] of masked) {
    it(`masks ${JSON.stringify(input)}`, () => {
      assert.equal(redactText(input), expected);
    });
  }

  for (const input of kept) {
    it(`keeps ${JSON.stringify(input)}`, () => {
      assert.equal(redactText(input), input);
    });
  }

  it('stays under 250 ms on 16 KB of prefixed code keys', () => {
    const input = 'x_code:'.repeat(2400);
    assert.ok(input.length >= 16000);
    const started = performance.now();
    redactText(input);
    assert.ok(performance.now() - started < 250);
  });
});

describe('spaced field names v6.37', () => {
  it('masks benign-looking prefixes before a secret word', () => {
    assert.deepEqual(
      sanitizePayload({
        'Monkey api_key': 'v0',
        'hotkey password': 'v1',
        'turkey session_id': 'v2',
        'donkey key': 'v3',
        'Hotkey pwd': 'v4',
      }),
      {
        'Monkey api_key': '*',
        'hotkey password': '*',
        'turkey session_id': '*',
        'donkey key': '*',
        'Hotkey pwd': '*',
      },
    );
  });

  it('keeps spaced names that are not secret', () => {
    const payload = { 'cache key': 'ok', monkey: 'ok', hotkey: 'ok' };
    assert.deepEqual(sanitizePayload(payload), { 'cache key': 'ok', monkey: 'ok', hotkey: 'ok' });
  });
});
