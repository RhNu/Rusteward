import assert from "node:assert/strict";
import test from "node:test";
import {
  archiveName,
  cacheKey,
  normalizeRef,
  parseChecksumFile,
  parseWaitTimeout,
  platformFor,
  releaseAssets,
  releaseTag,
  supportedTargets,
  validateManifest,
  validateSha,
} from "../src/model.mjs";

const SHA = "0123456789abcdef0123456789abcdef01234567";
const OTHER_SHA = "fedcba9876543210fedcba9876543210fedcba98";
const CHECKSUM = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

test("publication supports exactly the three agreed targets", () => {
  assert.deepEqual(supportedTargets(), [
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
  ]);
  const targets = supportedTargets();
  targets.length = 0;
  assert.equal(supportedTargets().length, 3);
});

test("supported runners select their executable and archive format", () => {
  assert.deepEqual(platformFor("Windows", "X64"), {
    target: "x86_64-pc-windows-msvc",
    binary: "cargo-dev.exe",
    archiveExtension: "zip",
  });
  assert.deepEqual(platformFor("Linux", "X64"), {
    target: "x86_64-unknown-linux-gnu",
    binary: "cargo-dev",
    archiveExtension: "tar.gz",
  });
  assert.deepEqual(platformFor("Linux", "ARM64"), {
    target: "aarch64-unknown-linux-gnu",
    binary: "cargo-dev",
    archiveExtension: "tar.gz",
  });
  for (const arch of ["amd64", "x86_64"]) {
    assert.equal(platformFor("linux", arch).target, "x86_64-unknown-linux-gnu");
  }
  assert.equal(platformFor("LINUX", "aarch64").target, "aarch64-unknown-linux-gnu");
});

test("unsupported operating systems and architectures fail explicitly", () => {
  for (const [os, arch] of [
    ["macOS", "X64"],
    ["Windows", "ARM64"],
    ["Linux", "X86"],
    ["Linux", "arm"],
    ["Linux", ""],
    [undefined, "X64"],
    ["Linux", null],
  ]) {
    assert.throws(() => platformFor(os, arch), /Unsupported runner platform/);
  }
});

test("commit identifiers require full hashes and normalize hexadecimal case", () => {
  assert.equal(validateSha(SHA.toUpperCase()), SHA);
  for (const value of [
    undefined,
    null,
    12,
    "",
    "abc123",
    SHA.slice(1),
    `${SHA}0`,
    `g${SHA.slice(1)}`,
    ` ${SHA}`,
  ]) {
    assert.throws(() => validateSha(value), /40 hexadecimal/);
  }
});

test("refs default to main and preserve valid branches and tags", () => {
  assert.equal(normalizeRef(), "main");
  for (const ref of [
    "main",
    "feature/install-cache",
    "v1.2.3",
    "refs/heads/main",
    "release-候选",
  ]) {
    assert.equal(normalizeRef(ref), ref);
  }
  assert.equal(normalizeRef(SHA.toUpperCase()), SHA);
});

test("refs reject empty values, URLs, revision expressions, and unsafe names", () => {
  for (const ref of [
    "",
    null,
    1,
    " main",
    "main\n",
    "main\u0000",
    "https://example.com/main",
    "git@example.com:main",
    "../main",
    "main..other",
    "main~1",
    "main^",
    "main@{1}",
    "-main",
    "@",
    "/main",
    "main/",
    "main//other",
    ".hidden",
    "branch/.hidden",
    "main.lock",
    "main.",
    "main\\other",
    "main?x",
    "main*x",
    "main[x",
    "main%2Fother",
  ]) {
    assert.throws(() => normalizeRef(ref), /Ref/);
  }
});

test("wait timeout defaults to 180 seconds and accepts bounded integers", () => {
  assert.equal(parseWaitTimeout(), 180);
  assert.equal(parseWaitTimeout(0), 0);
  assert.equal(parseWaitTimeout("45"), 45);
  assert.equal(parseWaitTimeout("900"), 900);
  for (const value of ["", null, false, -1, 901, 1.5, "1.5", "1e2", " 30", NaN, Infinity]) {
    assert.throws(() => parseWaitTimeout(value), /integer between 0 and 900/);
  }
});

test("publication and cache identifiers bind the commit and target", () => {
  assert.equal(releaseTag(SHA), "ci-0123456789abcdef0123456789abcdef01234567");
  assert.equal(
    archiveName(SHA, "x86_64-pc-windows-msvc"),
    "rusteward-0123456789abcdef0123456789abcdef01234567-x86_64-pc-windows-msvc.zip",
  );
  assert.equal(
    archiveName(SHA, "aarch64-unknown-linux-gnu"),
    "rusteward-0123456789abcdef0123456789abcdef01234567-aarch64-unknown-linux-gnu.tar.gz",
  );
  assert.equal(
    cacheKey(SHA, "x86_64-unknown-linux-gnu"),
    "rusteward-v1-0123456789abcdef0123456789abcdef01234567-x86_64-unknown-linux-gnu",
  );
  assert.notEqual(
    cacheKey(SHA, "x86_64-unknown-linux-gnu"),
    cacheKey(OTHER_SHA, "x86_64-unknown-linux-gnu"),
  );
  assert.notEqual(
    cacheKey(SHA, "x86_64-unknown-linux-gnu"),
    cacheKey(SHA, "aarch64-unknown-linux-gnu"),
  );
  assert.throws(() => archiveName(SHA, "../../other"), /Unsupported Rusteward target/);
  assert.throws(() => cacheKey("short", "x86_64-unknown-linux-gnu"), /40 hexadecimal/);
});

test("checksum files bind a single SHA256 to its archive with text or binary markers", () => {
  const name = "rusteward-example.tar.gz";
  assert.equal(parseChecksumFile(`${CHECKSUM}  ${name}\n`, name), CHECKSUM);
  assert.equal(parseChecksumFile(`${CHECKSUM.toUpperCase()} *${name}\r\n`, name), CHECKSUM);
  assert.equal(parseChecksumFile(`${CHECKSUM}  ${name}`, name), CHECKSUM);
  for (const contents of [
    "",
    null,
    `${CHECKSUM}  other.tar.gz\n`,
    `${CHECKSUM}  ${name}\n${CHECKSUM}  ${name}\n`,
    `${CHECKSUM}  ${name}\n\n`,
    `short  ${name}\n`,
    `${CHECKSUM} ${name}\n`,
    `${CHECKSUM}  ${name} \n`,
    `${CHECKSUM}  ../${name}\n`,
  ]) {
    assert.throws(() => parseChecksumFile(contents, name), /exactly one SHA256 entry/);
  }
});

function publication(overrides = {}) {
  const tag = "ci-0123456789abcdef0123456789abcdef01234567";
  const name = "rusteward-0123456789abcdef0123456789abcdef01234567-x86_64-unknown-linux-gnu.tar.gz";
  return {
    tag_name: tag,
    draft: false,
    assets: [
      {
        name,
        browser_download_url: `https://github.com/RhNu/Rusteward/releases/download/${tag}/${name}`,
      },
      {
        name: `${name}.sha256`,
        browser_download_url: `https://github.com/RhNu/Rusteward/releases/download/${tag}/${name}.sha256`,
      },
    ],
    ...overrides,
  };
}

test("release asset selection waits for a complete published platform pair", () => {
  const release = publication();
  assert.deepEqual(releaseAssets(release, SHA, "x86_64-unknown-linux-gnu"), {
    archive: release.assets[0],
    checksum: release.assets[1],
  });
  for (const unavailable of [
    undefined,
    null,
    publication({ draft: true }),
    publication({ assets: [] }),
    publication({ assets: undefined }),
    publication({ assets: [release.assets[0]] }),
    publication({ assets: [release.assets[1]] }),
  ]) {
    assert.equal(releaseAssets(unavailable, SHA, "x86_64-unknown-linux-gnu"), undefined);
  }
  assert.equal(releaseAssets(release, SHA, "aarch64-unknown-linux-gnu"), undefined);
});

test("release asset selection rejects unexpected tags, duplicate names, and untrusted URLs", () => {
  const release = publication();
  assert.throws(
    () => releaseAssets(publication({ tag_name: "ci-other" }), SHA, "x86_64-unknown-linux-gnu"),
    /Release tag/,
  );
  assert.throws(
    () => releaseAssets(publication({ assets: {} }), SHA, "x86_64-unknown-linux-gnu"),
    /must be an array/,
  );
  assert.throws(
    () =>
      releaseAssets(
        publication({ assets: [...release.assets, release.assets[0]] }),
        SHA,
        "x86_64-unknown-linux-gnu",
      ),
    /multiple assets/,
  );
  for (const url of [
    "https://example.com/tool.tar.gz",
    release.assets[0].browser_download_url.replace("RhNu/Rusteward", "Other/Rusteward"),
    `${release.assets[0].browser_download_url}?download=1`,
    release.assets[0].browser_download_url.replace("https:", "http:"),
  ]) {
    assert.throws(
      () =>
        releaseAssets(
          publication({
            assets: [{ ...release.assets[0], browser_download_url: url }, release.assets[1]],
          }),
          SHA,
          "x86_64-unknown-linux-gnu",
        ),
      /trusted download path/,
    );
  }
  assert.throws(
    () =>
      releaseAssets(
        publication({
          assets: [
            release.assets[0],
            { ...release.assets[1], browser_download_url: release.assets[0].browser_download_url },
          ],
        }),
        SHA,
        "x86_64-unknown-linux-gnu",
      ),
    /trusted download path/,
  );
});

function manifest(overrides = {}) {
  return {
    schema_version: 1,
    source_sha: SHA,
    target: "x86_64-unknown-linux-gnu",
    binary: "cargo-dev",
    binary_sha256: CHECKSUM,
    ...overrides,
  };
}

test("valid manifests return only the normalized installation metadata", () => {
  assert.deepEqual(
    validateManifest(
      manifest({
        source_sha: SHA.toUpperCase(),
        binary_sha256: CHECKSUM.toUpperCase(),
        extra: "ignored",
      }),
      SHA,
      "x86_64-unknown-linux-gnu",
    ),
    {
      schema_version: 1,
      source_sha: SHA,
      target: "x86_64-unknown-linux-gnu",
      binary: "cargo-dev",
      binary_sha256: CHECKSUM,
    },
  );
  assert.equal(
    validateManifest(
      manifest({
        target: "x86_64-pc-windows-msvc",
        binary: "cargo-dev.exe",
      }),
      SHA,
      "x86_64-pc-windows-msvc",
    ).binary,
    "cargo-dev.exe",
  );
});

test("manifests reject malformed metadata and mismatched artifact identities", () => {
  for (const data of [null, [], "manifest", undefined]) {
    assert.throws(
      () => validateManifest(data, SHA, "x86_64-unknown-linux-gnu"),
      /must be an object/,
    );
  }
  for (const override of [
    { schema_version: 2 },
    { schema_version: "1" },
    { source_sha: OTHER_SHA },
    { source_sha: "short" },
    { target: "aarch64-unknown-linux-gnu" },
    { binary: "cargo-dev.exe" },
    { binary_sha256: "short" },
    { binary_sha256: `g${CHECKSUM.slice(1)}` },
    { binary_sha256: null },
  ]) {
    assert.throws(() => validateManifest(manifest(override), SHA, "x86_64-unknown-linux-gnu"));
  }
});
