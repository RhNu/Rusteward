import * as core from "@actions/core";
import * as cache from "@actions/cache";
import * as tools from "@actions/tool-cache";
import { createHash } from "node:crypto";
import { chmod, copyFile, mkdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import {
  cacheKey,
  normalizeRef,
  parseChecksumFile,
  parseWaitTimeout,
  platformFor,
  releaseAssets,
  releaseTag,
  validateManifest,
  validateSha,
} from "./model.mjs";

const REPOSITORY = "RhNu/Rusteward";
const API = `https://api.github.com/repos/${REPOSITORY}`;

/** Request public repository metadata, keeping credentials on GitHub's API host. */
async function githubJson(resource, token, allowMissing = false, timeoutMs = 30_000) {
  const response = await fetch(`${API}/${resource}`, {
    headers: {
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "rusteward-install-action",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (response.status === 404 && allowMissing) return undefined;
  if (!response.ok) {
    throw new Error(
      `GitHub API request failed (${response.status}) for ${resource}. Check the ref, token, and API rate limit.`,
    );
  }
  return response.json();
}

/** Wait for this exact commit; neither another successful commit nor a source build is a substitute. */
async function waitForAssets(sha, target, token, timeout) {
  const deadline = Date.now() + timeout * 1000;
  let announced = false;
  const notReady = () =>
    new Error(
      `Prebuilt Rusteward ${sha} (${target}) is not ready after ${timeout}s. Check ${REPOSITORY}'s CI build and retry; installation requires a published package for this commit.`,
    );
  while (true) {
    if (announced && Date.now() >= deadline) throw notReady();
    // A slow final request must not extend the configured release-wait deadline.
    const requestTimeout =
      timeout === 0 ? 30_000 : Math.max(1, Math.min(30_000, deadline - Date.now()));
    let release;
    try {
      release = await githubJson(`releases/tags/${releaseTag(sha)}`, token, true, requestTimeout);
    } catch (error) {
      if (timeout > 0 && Date.now() >= deadline) throw notReady();
      throw error;
    }
    const assets = releaseAssets(release, sha, target);
    if (assets) return assets;
    if (Date.now() >= deadline) throw notReady();
    if (!announced) {
      core.info(`Waiting up to ${timeout}s for ${releaseTag(sha)} (${target}) to be published.`);
      announced = true;
    }
    core.debug(`Prebuilt release is still unavailable for ${sha}.`);
    await sleep(Math.min(5000, deadline - Date.now()));
  }
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

/** Validate an extracted installation before it becomes executable or enters a cache. */
async function verifyInstallation(directory, sha, target) {
  const manifest = validateManifest(
    JSON.parse(await readFile(join(directory, "manifest.json"), "utf8")),
    sha,
    target,
  );
  const actual = digest(await readFile(join(directory, manifest.binary)));
  if (actual !== manifest.binary_sha256)
    throw new Error("Installed binary SHA256 does not match its manifest.");
  return manifest;
}

/** Corrupt/incomplete caches are misses; they must never supply the executable. */
async function reusableInstallation(directory, sha, target) {
  try {
    await verifyInstallation(directory, sha, target);
    return true;
  } catch (error) {
    if (error.code !== "ENOENT") core.warning(`Ignoring invalid Rusteward cache: ${error.message}`);
    return false;
  }
}

/** Install a verified archive into its immutable per-commit directory. */
async function downloadInstallation(directory, sha, platform, assets) {
  const checksumPath = await tools.downloadTool(assets.checksum.browser_download_url);
  const expected = parseChecksumFile(await readFile(checksumPath, "utf8"), assets.archive.name);
  core.info(`Downloading ${assets.archive.name}.`);
  const archivePath = await tools.downloadTool(assets.archive.browser_download_url);
  if (digest(await readFile(archivePath)) !== expected)
    throw new Error("Downloaded archive SHA256 does not match the published checksum.");
  const extracted =
    platform.archiveExtension === "zip"
      ? await tools.extractZip(archivePath)
      : await tools.extractTar(archivePath);
  await verifyInstallation(extracted, sha, platform.target);
  // The directory is always derived from a validated full SHA and a supported target.
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  // Runner temp and tool-cache can live on different volumes, particularly on Windows.
  await copyFile(join(extracted, platform.binary), join(directory, platform.binary));
  await copyFile(join(extracted, "manifest.json"), join(directory, "manifest.json"));
}

/** Resolve once, restore or download, and expose cargo-dev without altering the project toolchain. */
async function install() {
  const started = Date.now();
  const ref = normalizeRef(core.getInput("ref") || undefined);
  const timeout = parseWaitTimeout(core.getInput("wait-timeout") || undefined);
  const useCache = core.getBooleanInput("cache");
  const token = core.getInput("token");
  if (token) core.setSecret(token);
  const platform = platformFor(
    process.env.RUNNER_OS || (process.platform === "win32" ? "Windows" : process.platform),
    process.env.RUNNER_ARCH || process.arch,
  );
  const sha = validateSha((await githubJson(`commits/${encodeURIComponent(ref)}`, token)).sha);
  core.info(`Resolved ${REPOSITORY}@${ref} to ${sha}; selected ${platform.target}.`);
  const directory = join(
    process.env.RUNNER_TOOL_CACHE || process.env.RUNNER_TEMP || tmpdir(),
    "rusteward",
    sha,
    platform.target,
  );
  const key = cacheKey(sha, platform.target);
  let hit = await reusableInstallation(directory, sha, platform.target);
  let restored = false;
  const cacheAvailable = useCache && cache.isFeatureAvailable();
  if (useCache && !cacheAvailable)
    core.warning(
      "GitHub Actions cache is unavailable; continuing with a verified download or runner-local cache.",
    );
  if (!hit && cacheAvailable) {
    // Clear an invalid local directory before merging restored files into it.
    await rm(directory, { recursive: true, force: true });
    try {
      restored = (await cache.restoreCache([directory], key, [])) === key;
      hit = restored && (await reusableInstallation(directory, sha, platform.target));
      core.info(hit ? `Restored ${key}.` : `Cache miss for ${key}.`);
    } catch (error) {
      core.warning(`Could not restore Rusteward cache: ${error.message}`);
    }
  } else if (hit) {
    core.info(`Reusing validated runner-local Rusteward ${sha}.`);
  }
  if (!hit) {
    const assets = await waitForAssets(sha, platform.target, token, timeout);
    await downloadInstallation(directory, sha, platform, assets);
  }
  if (platform.binary === "cargo-dev") await chmod(join(directory, platform.binary), 0o755);
  if (!hit && !restored && cacheAvailable) {
    try {
      const id = await cache.saveCache([directory], key);
      core.info(`Saved Rusteward cache ${key} (${id}).`);
    } catch (error) {
      core.warning(`Rusteward is installed, but its cache could not be saved: ${error.message}`);
    }
  }
  core.addPath(directory);
  core.setOutput("sha", sha);
  core.setOutput("target", platform.target);
  core.setOutput("cache-hit", hit);
  core.setOutput("path", directory);
  core.info(
    `Installed cargo-dev from ${sha} for ${platform.target} in ${((Date.now() - started) / 1000).toFixed(1)}s (cache hit: ${hit}).`,
  );
}

install().catch((error) => {
  core.setFailed(error instanceof Error ? error.message : String(error));
});
