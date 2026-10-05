const TARGETS = new Map([
  ["x86_64-pc-windows-msvc", { binary: "cargo-dev.exe", archiveExtension: "zip" }],
  ["x86_64-unknown-linux-gnu", { binary: "cargo-dev", archiveExtension: "tar.gz" }],
  ["aarch64-unknown-linux-gnu", { binary: "cargo-dev", archiveExtension: "tar.gz" }],
]);

function targetDetails(target) {
  const details = TARGETS.get(target);
  if (!details) {
    throw new Error(`Unsupported Rusteward target: ${String(target)}`);
  }
  return details;
}

/** Share the publication catalog without exposing its mutable backing map. */
export function supportedTargets() {
  return [...TARGETS.keys()];
}

/** Map the runner environment to a supported precompiled binary. */
export function platformFor(os, arch) {
  const normalizedOs = typeof os === "string" ? os.toLowerCase() : "";
  const normalizedArch = typeof arch === "string" ? arch.toLowerCase() : "";
  const x64 = ["x64", "amd64", "x86_64"].includes(normalizedArch);
  const arm64 = ["arm64", "aarch64"].includes(normalizedArch);
  let target;
  if (normalizedOs === "windows" && x64) {
    target = "x86_64-pc-windows-msvc";
  } else if (normalizedOs === "linux" && x64) {
    target = "x86_64-unknown-linux-gnu";
  } else if (normalizedOs === "linux" && arm64) {
    target = "aarch64-unknown-linux-gnu";
  } else {
    throw new Error(`Unsupported runner platform: ${String(os)} / ${String(arch)}`);
  }
  return { target, ...targetDetails(target) };
}

/** Require the full immutable commit identifier used by artifacts and caches. */
export function validateSha(value) {
  if (typeof value !== "string" || !/^[0-9a-f]{40}$/i.test(value)) {
    throw new Error("Source SHA must contain exactly 40 hexadecimal characters");
  }
  return value.toLowerCase();
}

/** Accept a Git ref without permitting URL, revision-expression, or option syntax. */
export function normalizeRef(value) {
  if (value === undefined) {
    return "main";
  }
  if (typeof value !== "string" || value.length === 0) {
    throw new Error("Ref must be a nonempty branch, tag, or full commit SHA");
  }
  if (/^[0-9a-f]{40}$/i.test(value)) {
    return validateSha(value);
  }

  // Git disallows these forms; leading option syntax is excluded as well.
  const components = value.split("/");
  const unsafe =
    /[\s\u0000-\u001f\u007f-\u009f~^:?*\[\\%]/u.test(value) ||
    value.startsWith("-") ||
    value === "@" ||
    value.includes("..") ||
    value.includes("@{") ||
    components.some(
      (part) => part.length === 0 || part.startsWith(".") || part.endsWith(".lock"),
    ) ||
    value.endsWith(".");
  if (unsafe) {
    throw new Error("Ref contains unsupported or unsafe Git ref syntax");
  }
  return value;
}

/** Bound waiting for a fresh main build so installation cannot wait indefinitely. */
export function parseWaitTimeout(value) {
  if (value === undefined) {
    return 180;
  }
  if (
    (typeof value !== "number" && typeof value !== "string") ||
    (typeof value === "string" && !/^\d+$/.test(value))
  ) {
    throw new Error("Wait timeout must be an integer between 0 and 900 seconds");
  }
  const timeout = Number(value);
  if (!Number.isInteger(timeout) || timeout < 0 || timeout > 900) {
    throw new Error("Wait timeout must be an integer between 0 and 900 seconds");
  }
  return timeout;
}

/** Keep each source commit's publication independently addressable. */
export function releaseTag(sha) {
  return `ci-${validateSha(sha)}`;
}

/** Select an archive using the source commit and the supported platform. */
export function archiveName(sha, target) {
  const { archiveExtension } = targetDetails(target);
  return `rusteward-${validateSha(sha)}-${target}.${archiveExtension}`;
}

/** Isolate installed binaries by immutable commit and platform. */
export function cacheKey(sha, target) {
  targetDetails(target);
  return `rusteward-v1-${validateSha(sha)}-${target}`;
}

/** Bind a conventional SHA256 checksum entry to the requested archive name. */
export function parseChecksumFile(contents, expectedArchive) {
  const entry =
    typeof contents === "string"
      ? /^([0-9a-f]{64}) [ *]([^\r\n]+)(?:\r?\n)?$/i.exec(contents)
      : null;
  if (!entry || entry[2] !== expectedArchive) {
    throw new Error(
      "Checksum file must contain exactly one SHA256 entry for the requested archive",
    );
  }
  return entry[1].toLowerCase();
}

/** Select a complete publication and reject assets outside the trusted download path. */
export function releaseAssets(release, sha, target) {
  const tag = releaseTag(sha);
  const name = archiveName(sha, target);
  if (release === undefined || release === null || release.draft === true) {
    return undefined;
  }
  if (typeof release !== "object" || Array.isArray(release) || release.tag_name !== tag) {
    throw new Error("Release tag does not match the requested source commit");
  }
  if (release.assets === undefined) {
    return undefined;
  }
  if (!Array.isArray(release.assets)) {
    throw new Error("Release assets must be an array");
  }
  const selected = {};
  for (const [kind, filename] of [
    ["archive", name],
    ["checksum", `${name}.sha256`],
  ]) {
    const matches = release.assets.filter((asset) => asset?.name === filename);
    if (matches.length > 1) {
      throw new Error(`Release contains multiple assets named ${filename}`);
    }
    if (matches.length === 0) {
      continue;
    }
    const asset = matches[0];
    const expectedUrl = `https://github.com/RhNu/Rusteward/releases/download/${tag}/${filename}`;
    if (asset.browser_download_url !== expectedUrl) {
      throw new Error(`Release asset URL does not match the trusted download path for ${filename}`);
    }
    selected[kind] = asset;
  }
  return selected.archive && selected.checksum ? selected : undefined;
}

/** Bind a download manifest to the requested source, platform, and executable. */
export function validateManifest(data, sha, target) {
  const sourceSha = validateSha(sha);
  const { binary } = targetDetails(target);
  if (data === null || typeof data !== "object" || Array.isArray(data)) {
    throw new Error("Artifact manifest must be an object");
  }
  if (data.schema_version !== 1) {
    throw new Error("Artifact manifest schema_version must be 1");
  }
  if (validateSha(data.source_sha) !== sourceSha) {
    throw new Error("Artifact manifest source SHA does not match the requested commit");
  }
  if (data.target !== target) {
    throw new Error("Artifact manifest target does not match the requested platform");
  }
  if (data.binary !== binary) {
    throw new Error("Artifact manifest binary does not match the requested platform");
  }
  if (typeof data.binary_sha256 !== "string" || !/^[0-9a-f]{64}$/i.test(data.binary_sha256)) {
    throw new Error(
      "Artifact manifest binary_sha256 must contain exactly 64 hexadecimal characters",
    );
  }
  return {
    schema_version: 1,
    source_sha: sourceSha,
    target,
    binary,
    binary_sha256: data.binary_sha256.toLowerCase(),
  };
}
