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
      { token: ['*', ['*']], apiKey: { '*': '*', '*#2': '*' }, list: ['ok'] },
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
      password: { '*': true, '*#2': null, '*#3': { '*': false, '*#2': '*' } },
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

describe('redactText doubled equals and long or punctuated prefixes', () => {
  const exact: ReadonlyArray<readonly [string, string]> = [
    ['password==""\'\'""\'\'ZqSecret123', 'password==*'],
    ["code==''\"\"''\"\";", 'code==*'],
    ['pwd==abcRET1', 'pwd==*'],
    ['https://admin:hunter2@bücher.example/x', 'https://bücher.example/x'],
    ['see //password:aJ@+QvZ7 end', 'see //password:*'],
  ];

  for (const [input, expected] of exact) {
    it(`turns ${JSON.stringify(input)} into ${JSON.stringify(expected)}`, () => {
      assert.equal(redactText(input), expected);
      assert.equal(redactText(expected), expected);
    });
  }

  for (const input of ['a == b', 'x==y', 'password===x1', 'password== x1', 'readyState=complete', 'game_state=lobby', 'iceGatheringState=complete', 'dial_code=33']) {
    it(`keeps ${JSON.stringify(input)}`, () => {
      assert.equal(redactText(input), input);
    });
  }

  for (const [input, marker] of [
    ['device_state=ZqMark91', 'ZqMark91'],
    ['authRequestState=ZqMark91', 'ZqMark91'],
    ['phone_code=ZqMark91', 'ZqMark91'],
    ['phoneCode=ZqMark91', 'ZqMark91'],
    ['a'.repeat(300) + '_password=hunter2abc', 'hunter2abc'],
    ['a'.repeat(300) + '_otp_code: 834201', '834201'],
  ] as const) {
    it(`masks ${JSON.stringify(input.length > 60 ? `${input.slice(0, 12)}...${input.slice(-24)}` : input)}`, () => {
      assert.ok(!redactText(input).includes(marker), redactText(input));
    });
  }

  it('keeps a long benign prefix before a non-secret key', () => {
    const input = 'a'.repeat(300) + '_monkey=3';
    assert.equal(redactText(input), input);
  });

  it('stays under 250 ms on 16 KB behind a long prefix and a run of equals', () => {
    for (const input of ['a'.repeat(16000) + '_monkey=3', 'a'.repeat(16000) + '_password=hunter2abc', 'password' + '='.repeat(16000) + 'x']) {
      const started = performance.now();
      redactText(input);
      assert.ok(performance.now() - started < 250);
    }
  });
});

describe('field names with punctuation and long prefixes', () => {
  const valueOf = (name: string): unknown => {
    const out = sanitizeItem({ type: 'log', meta: {}, payload: { [name]: 'ZqVal1' } } as never);
    assert.ok(out);
    return Object.values((out as unknown as { payload: Record<string, unknown> }).payload)[0];
  };

  for (const name of ['login(form)/password', 'user@host/password', 'form#login/token', 'a+b/secret', '100%/token', 'a'.repeat(80) + '_password']) {
    it(`masks ${JSON.stringify(name.length > 40 ? `${name.slice(0, 4)}...${name.slice(-9)}` : name)}`, () => {
      assert.equal(valueOf(name), '*');
    });
  }

  for (const name of ['cache key', 'i18n:key', 'homeCacheKey', 'a'.repeat(80) + '_monkey']) {
    it(`keeps ${JSON.stringify(name.length > 40 ? `${name.slice(0, 4)}...${name.slice(-7)}` : name)}`, () => {
      assert.equal(valueOf(name), 'ZqVal1');
    });
  }
});

const textsOf = (root: unknown): string[] => {
  const seen = new Set<unknown>();
  const out: string[] = [];
  const visit = (value: unknown): void => {
    if (typeof value === 'string') {
      out.push(value);
      return;
    }
    if (typeof value !== 'object' || value === null || seen.has(value)) return;
    seen.add(value);
    for (const [key, inner] of Object.entries(value)) {
      out.push(key);
      visit(inner);
    }
  };
  visit(root);
  return out;
};

describe('sanitizeItem copies and the Faro session meta', () => {
  type Sanitized = { meta: Record<string, any>; payload: Record<string, any> };
  const run = (meta: object, payload: object): Sanitized => {
    const out = sanitizeItem({ type: 'log', meta, payload } as never);
    assert.ok(out);
    return out as unknown as Sanitized;
  };

  it('does not mutate the item or anything reachable from it', () => {
    const session = { id: 'sess-123', attributes: { plan: 'pro' } };
    const meta = { session, user: { email: 'jane@corp.io' } };
    const payload = { context: { password: 'hunter2SECRETzq', note: 'a@b.com' }, list: ['a@b.com'] };
    const before = structuredClone({ meta, payload });
    const out = run(meta, payload);
    assert.deepEqual({ meta, payload }, before);
    assert.notEqual(out.meta['session'], session);
    assert.equal(out.payload['context']['password'], '*');
  });

  it('keeps the session id and attributes of the Faro meta', () => {
    const { meta } = run({ session: { id: 'sess-123', attributes: { plan: 'pro' } } }, {});
    assert.deepEqual(meta['session'], { id: 'sess-123', attributes: { plan: 'pro' } });
  });

  it('masks a string, number or bigint directly under session', () => {
    const { payload } = run({}, { session: 'abc123SECRET' });
    assert.equal(payload['session'], '*');
    assert.equal(run({}, { Session: 48291234 }).payload['Session'], '*');
    assert.equal(run({}, { session: 48291234n }).payload['session'], '*');
  });

  it('still masks the other session fields', () => {
    const { payload } = run({}, { session_id: 'abc123SECRET', sessionId: 'abc123SECRET', session_token: 'abc123SECRET', sessionKey: 'abc123SECRET' });
    assert.deepEqual(Object.values(payload), ['*', '*', '*', '*']);
  });

  it('walks a non-extensible object with a secret and colliding keys without touching it', () => {
    const data = Object.preventExtensions({ password: 'hunter2SECRETzq', 'a@b.com': 1, 'c@d.com': 2, keep: 'ok' });
    const before = structuredClone(data);
    const { payload } = run({}, { data });
    assert.deepEqual(data, before);
    assert.equal(payload['data']['password'], '*');
    assert.equal(payload['data']['keep'], 'ok');
    assert.equal(Object.keys(payload['data']).length, 4);
  });

  it('masks every leaf of an object that holds itself under a secret field', () => {
    const loop: Record<string, unknown> = {};
    loop['password'] = loop;
    loop['note'] = 'hunter2SECRETzq';
    const { payload } = run({}, { loop });
    assert.ok(textsOf(payload['loop']).every((text) => text.startsWith('*')));
    const reversed: Record<string, unknown> = { note: 'hunter2SECRETzq' };
    reversed['password'] = reversed;
    assert.ok(!textsOf(run({}, { reversed }).payload['reversed']).some((text) => text.includes('hunter2')));
  });

  it('keeps token counts and masks bare tokens', () => {
    const { payload } = run({}, { usage: { inputTokens: 12, max_tokens: 100, total_tokens: 7, OUTPUT_TOKENS: 3, 'prompt-tokens': 5 }, tokens: ['abc123SECRET'], token: 42 });
    assert.deepEqual(payload['usage'], { inputTokens: 12, max_tokens: 100, total_tokens: 7, OUTPUT_TOKENS: 3, 'prompt-tokens': 5 });
    assert.deepEqual(payload['tokens'], ['*']);
    assert.equal(payload['token'], '*');
  });
});

describe('redactText chained colon values, slashed passwords, nbsp values and quote chains', () => {
  const exact: ReadonlyArray<readonly [string, string]> = [
    ['client_secret: monkeySecret=\nfoo=1', 'client_secret: *'],
    ['Error key: key: \n"Token X" end', 'Error key: * end'],
    ['DATABASE_URL=postgres://app:pa/ss#1@db.internal:5432/app', 'DATABASE_URL=postgres://db.internal:5432/app'],
    ['redis://default:ab/cd?ef@cache:6379/0', 'redis://cache:6379/0'],
    ['https://u:123?x@h.com/a', 'https://u:123'],
    ['password: correct horse', 'password: *'],
    ['password=correct horse', 'password=*'],
    ['password=a"b"c"d"e"f"g"h"hunter2abc', 'password=*'],
    ["pwd=^\"\"\"\"''''k", 'pwd=*'],
    ["auth=&\"\"''''\"\"'", 'auth=*'],
  ];

  for (const [input, expected] of exact) {
    it(`turns ${JSON.stringify(input)} into ${JSON.stringify(expected)}`, () => {
      assert.equal(redactText(input), expected);
    });
  }

  for (const input of ['password=a"b"c"d"e"f"g"h"hunter2abc', "pwd=^\"\"\"\"''''k", "auth=&\"\"''''\"\"'"]) {
    it(`is idempotent on ${JSON.stringify(input)}`, () => {
      assert.equal(redactText(redactText(input)), redactText(input));
    });
  }

  for (let code = 0x2000; code <= 0x200a; code++) {
    it(`masks a value split by U+${code.toString(16).toUpperCase()}`, () => {
      assert.equal(redactText(`password: correct${String.fromCharCode(code)}horse`), 'password: *');
    });
  }

  it('stays under 250 ms on unbalanced brackets', () => {
    for (const input of ['x['.repeat(8000), 'a]'.repeat(8000)]) {
      const started = performance.now();
      redactText(input);
      assert.ok(performance.now() - started < 250);
    }
  });
});

describe('sanitizeItem keys under secret fields', () => {
  const marker = 'ghp_Zq9realTokenValue';
  const run = (payload: object): Record<string, any> => sanitizePayload(payload);

  it('masks keys, values and shape under a secret field', () => {
    const out = run({ token: { [marker]: { a: 1 } } });
    assert.deepEqual(out, { token: { '*': { '*': '*' } } });
    assert.ok(!JSON.stringify(out).includes(marker));
  });

  it('numbers colliding keys under a secret field', () => {
    const out = run({ token: { 'ghp_Zq9realTokenValue1': 'x', 'ghp_Zq9realTokenValue2': 'y' } });
    assert.deepEqual(Object.keys(out['token']), ['*', '*#2']);
  });

  it('keeps null and booleans under a secret subtree', () => {
    assert.deepEqual(run({ password: { enabled: true, off: false, value: null } }), { password: { '*': true, '*#2': false, '*#3': null } });
  });

  it('leaves keys outside secret subtrees unchanged', () => {
    const payload = { settings: { theme: 'dark', nested: { count: 3 } } };
    assert.deepEqual(run(payload), payload);
  });

  it('masks every leaf and key of a self-referencing secret object', () => {
    const b: Record<string, unknown> = {};
    b['password'] = b;
    b[marker] = 'x';
    b['note'] = marker;
    const out = run({ b });
    assert.ok(!textsOf(out).some((text) => text.includes(marker)));
    assert.deepEqual([...new Set(textsOf(out['b']))], ['*', '*#2', '*#3']);
  });

  it('does not mutate the input', () => {
    const payload = { token: { [marker]: { a: 1 } } };
    const before = structuredClone(payload);
    run(payload);
    assert.deepEqual(payload, before);
  });
});

describe('credential hosts, cookies, bearer chains and code words', () => {
  const rows: ReadonlyArray<readonly [string, string]> = [
    ['//password:12345/next', '//password:*'],
    ['see //token:8342/x', 'see //token:*'],
    ['//auth:8080/x', '//auth:8080/x'],
    ['//db.internal:5432/app', '//db.internal:5432/app'],
    ['Cookie: theme=dark; sid=abc123', 'Cookie: *'],
    ['Set-Cookie: sid=abc123; Path=/', 'Set-Cookie: *'],
    ['cookie=abc123', 'cookie=*'],
    ['cookie_consent=accepted', 'cookie_consent=accepted'],
    ['cookieConsent: yes', 'cookieConsent: yes'],
    ['a'.repeat(60) + '_monkey=3', 'a'.repeat(60) + '_monkey=3'],
    ['a'.repeat(317) + '_monkey=3', 'a'.repeat(317) + '_monkey=3'],
    ['a'.repeat(300) + '_password=hunter2abc', 'a'.repeat(300) + '_password=*'],
    ['a'.repeat(300) + '_otp_code: 834201', 'a'.repeat(300) + '_otp_code: *'],
    ['Authorization: Bearer abcdefgh&otp_code: 834201', 'Authorization: Bearer *&otp_code: *'],
    [
      'Authorization: Bearer abcdefgh&id=1&password=Basic ZqSECRET9',
      'Authorization: Bearer *&password=*',
    ],
    ['password: correct\vhorse', 'password: *'],
    ['password: correct\fhorse', 'password: *'],
    ['versionCode=42', 'versionCode=42'],
    ['roomCode=AB12', 'roomCode=AB12'],
    ['combatState=idle', 'combatState=idle'],
    ['boardState=x', 'boardState=x'],
    ['team_code=x', 'team_code=*'],
  ];

  for (const [input, output] of rows) {
    it(`redacts ${JSON.stringify(input.slice(0, 48))} to ${JSON.stringify(output.slice(-24))}`, () => {
      assert.equal(redactText(input), output);
    });
  }
});

describe('secret reachability does not depend on key order', () => {
  const MARKER = 'hunter2SECRETzq';

  interface Walked {
    meta: { session?: unknown };
    payload: {
      plain?: unknown;
      secret?: unknown;
      password?: object;
      reversed?: object;
      context?: { session?: unknown };
    };
  }

  function walked(payload: object, meta: object = {}): Walked {
    const out = sanitizeItem({ type: 'log', meta, payload } as unknown as TransportItem);
    assert.ok(out);
    return out as unknown as Walked;
  }

  function containsMarker(value: unknown, seen = new WeakSet<object>()): boolean {
    if (typeof value === 'string') return value.includes(MARKER);
    if (value === null || typeof value !== 'object') return false;
    if (seen.has(value)) return false;
    seen.add(value);
    return Object.entries(value).some(
      ([key, entry]) => key.includes(MARKER) || containsMarker(entry, seen),
    );
  }

  for (const keys of [
    ['password', 'reversed'],
    ['reversed', 'password'],
  ]) {
    it(`masks a cycle through a secret field with keys in the order ${keys.join(', ')}`, () => {
      const ring: { note: string; password?: unknown } = { note: MARKER };
      ring.password = ring;
      const payload: Record<string, unknown> = {};
      for (const key of keys) payload[key] = ring;
      const out = walked(payload).payload;
      assert.equal(containsMarker(out), false);
      assert.deepEqual(Object.keys(out.reversed ?? {}), Object.keys(out.password ?? {}));
    });
  }

  for (const keys of [
    ['token', 'loop'],
    ['loop', 'token'],
  ]) {
    it(`masks a loop shared with a token field with keys in the order ${keys.join(', ')}`, () => {
      const loop: { note: string; password?: unknown } = { note: MARKER };
      loop.password = loop;
      const payload: Record<string, unknown> = {};
      for (const key of keys) payload[key] = loop;
      assert.equal(containsMarker(walked(payload).payload), false);
    });
  }

  for (const keys of [
    ['plain', 'secret'],
    ['secret', 'plain'],
  ]) {
    it(`masks a container shared between a plain and a secret field, order ${keys.join(', ')}`, () => {
      const shared = { v: MARKER };
      const payload: Record<string, unknown> = {};
      for (const key of keys) payload[key] = shared;
      const out = walked(payload).payload;
      assert.deepEqual(out.plain, { '*': '*' });
      assert.deepEqual(out.secret, { '*': '*' });
    });
  }

  it('masks everything under a session key outside meta', () => {
    const out = walked({ context: { session: { value: MARKER, cookie: MARKER } } }).payload;
    assert.deepEqual(out.context?.session, { '*': '*', '*#2': '*' });
  });

  it('masks a session key nested in meta.session', () => {
    const out = walked({}, { session: { id: 's1', nested: { session: { value: MARKER } } } }).meta;
    assert.equal(containsMarker(out), false);
  });

  it('masks cookie fields', () => {
    const out = walked({ cookie: MARKER, headers: { Cookie: 'a=' + MARKER } }).payload;
    assert.deepEqual(out, { cookie: '*', headers: { Cookie: '*' } });
  });

  it('keeps meta.session and masks secrets inside it', () => {
    const session = { id: 'sess-1', attributes: { a: 'b' }, token: MARKER };
    const out = walked({}, { session }).meta;
    assert.deepEqual(out.session, { id: 'sess-1', attributes: { a: 'b' }, token: '*' });
  });

  it('masks meta.session when a secret field also reaches it', () => {
    const session = { id: 'sess-1' };
    const out = walked({ token: session }, { session }).meta;
    assert.deepEqual(out.session, { '*': '*' });
  });
});

describe('redactText primary and query keys, narrowed state words', () => {
  const masked: ReadonlyArray<readonly [string, string]> = [
  ['auth_request_state=SECRET99', 'auth_request_state=*'],
  ['oauth_client_state=SECRET99', 'oauth_client_state=*'],
  ['api_key_code=SECRET99', 'api_key_code=*'],
  ['orderState=paid', 'orderState=*'],
  ['x_keycode=1', 'x_keycode=*'],
  ['cosmosPrimaryKey=SECRET99', 'cosmosPrimaryKey=*'],
  ['storageAccountPrimaryKey: SECRET99', 'storageAccountPrimaryKey: *'],
  ['searchAdminQueryKey=SECRET99', 'searchAdminQueryKey=*'],
  ['cosmos_primary_key=SECRET99', 'cosmos_primary_key=*'],
  ['email=a@b.com', '*'],
  ];
  const kept: readonly string[] = [
  'keyCode=13',
  'key_code=13',
  'readyState=complete',
  'game_state=lobby',
  ];

  for (const [input, expected] of masked) {
    it(`masks ${JSON.stringify(input)} as ${JSON.stringify(expected)}`, () => {
      assert.equal(redactText(input), expected);
    });
  }

  for (const input of kept) {
    it(`keeps ${JSON.stringify(input)}`, () => {
      assert.equal(redactText(input), input);
    });
  }

  it('drops the password of a host-relative userinfo with a query', () => {
    assert.ok(!redactText('//host:abc/path?email=a@b.com').includes('abc'));
  });

  for (const unit of ['x_request_state=', 'cosmosPrimaryKey=', 'x_client_state=', 'x_keycode=']) {
    it(`stays under 250 ms on 16 KB of ${unit}`, () => {
      const input = unit.repeat(Math.ceil(16384 / unit.length));
      const started = performance.now();
      redactText(input);
      assert.ok(performance.now() - started < 250);
    });
  }
});

describe('field names: primary and query keys, narrowed state words', () => {
  const payloadOf = (payload: object): Record<string, unknown> => {
    const out = sanitizeItem({ type: 'log', meta: {}, payload } as unknown as TransportItem);
    return (out as unknown as { payload: Record<string, unknown> }).payload;
  };

  for (const name of ['auth_request_state', 'oauth_client_state', 'api_key_code', 'orderState', 'x_keycode', 'cosmosPrimaryKey', 'storageAccountPrimaryKey', 'searchAdminQueryKey', 'cosmos_primary_key']) {
    it(`masks the field ${name}`, () => {
      assert.deepEqual(payloadOf({ [name]: 'SECRET99' }), { [name]: '*' });
    });
  }

  for (const name of ['primaryKey', 'primary_key', 'db.primaryKey', 'queryKey', 'keyCode', 'key_code', 'readyState', 'game_state']) {
    it(`keeps the field ${name}`, () => {
      assert.deepEqual(payloadOf({ [name]: 'ok' }), { [name]: 'ok' });
    });
  }

  it('keeps a primaryKey nested under a plain parent', () => {
    assert.deepEqual(payloadOf({ db: { primaryKey: 'ok', queryKey: 'ok' } }), {
      db: { primaryKey: 'ok', queryKey: 'ok' },
    });
  });
});

describe('bearer tail stops before a following secret key v6.44', () => {
  const rows: ReadonlyArray<readonly [string, string]> = [
    [
      'Authorization: Bearer abcdefgh&user=passwd: ZqSECRET9',
      'Authorization: Bearer *=passwd: *',
    ],
    [
      'Authorization: Bearer abcdefgh&id=password: ZqSECRET9',
      'Authorization: Bearer *=password: *',
    ],
    [
      'Authorization: Bearer abcdefgh:token: ZqSECRET9',
      'Authorization: Bearer *:token: *',
    ],
    [
      'Authorization: Bearer abcdefgh=otp_code: ZqSECRET9',
      'Authorization: Bearer *=otp_code: *',
    ],
  ];

  for (const [input, output] of rows) {
    it(`redacts ${JSON.stringify(input)}`, () => {
      assert.equal(redactText(input), output);
    });
  }
});

describe('secret words deep inside long keys v6.45', () => {
  const rows: ReadonlyArray<readonly [string, string]> = [
    ['b'.repeat(100) + 'Password=hunter2', 'b'.repeat(100) + 'Password=*'],
    ['x_' + 'c'.repeat(100) + 'Token: abc', 'x_' + 'c'.repeat(100) + 'Token: *'],
    ['a'.repeat(317) + '_monkey=3', 'a'.repeat(317) + '_monkey=3'],
    ['b'.repeat(100) + '_error_code=500', 'b'.repeat(100) + '_error_code=500'],
    ['b'.repeat(100) + 'ErrorCode=500', 'b'.repeat(100) + 'ErrorCode=500'],
  ];

  rows.forEach(([input, output], index) => {
    it(`redacts row ${index}`, () => {
      assert.equal(redactText(input), output);
    });
  });

  const adversarial: ReadonlyArray<string> = [
    '[a'.repeat(8000) + ':',
    'a-'.repeat(8000) + ':',
  ];

  adversarial.forEach((input, index) => {
    it(`stays under 250 ms on 16 KB of adversarial keys ${index}`, () => {
      const started = performance.now();
      redactText(input);
      assert.ok(performance.now() - started < 250);
    });
  });
});

describe('field names: group, object and lookup keys v6.46', () => {
  const payloadOf = (payload: object): Record<string, unknown> => {
    const out = sanitizeItem({ type: 'log', meta: {}, payload } as unknown as TransportItem);
    return (out as unknown as { payload: Record<string, unknown> }).payload;
  };

  for (const name of ['wpaGroupKey', 'mlsGroupKey', 's3ObjectKey']) {
    it(`masks the field ${name}`, () => {
      assert.deepEqual(payloadOf({ [name]: 'SECRET99' }), { [name]: '*' });
    });
  }

  for (const name of ['groupKey', 'group_key', 'db.groupKey', 'objectKey', 'foreignKey', 'uniqueKey', 'lookupKey', 'homeCacheKey', 'partitionKey', 'cosmosPartitionKey']) {
    it(`keeps the field ${name}`, () => {
      assert.deepEqual(payloadOf({ [name]: 'ok' }), { [name]: 'ok' });
    });
  }
});
