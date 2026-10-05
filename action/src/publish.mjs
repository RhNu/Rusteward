import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import {
  archiveName,
  parseChecksumFile,
  releaseAssets,
  releaseTag,
  supportedTargets,
  validateSha,
} from "./model.mjs";

/** Publish only complete three-platform releases; failed uploads remain invisible drafts. */
async function publish() {
  const sha = validateSha(process.env.GITHUB_SHA);
  const repository = process.env.GITHUB_REPOSITORY;
  const token = process.env.GH_TOKEN;
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository || ""))
    throw new Error("GITHUB_REPOSITORY is invalid.");
  if (repository !== "RhNu/Rusteward")
    throw new Error("Prebuilt publication is restricted to RhNu/Rusteward.");
  if (!token) throw new Error("GH_TOKEN is required to publish prebuilt releases.");
  const tag = releaseTag(sha);
  const api = `https://api.github.com/repos/${repository}`;
  const headers = {
    Authorization: `Bearer ${token}`,
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "rusteward-prebuilt-publisher",
  };
  async function request(resource, method = "GET", data, allowMissing = false) {
    const response = await fetch(`${api}/${resource}`, {
      method,
      headers: { ...headers, "Content-Type": "application/json" },
      body: data === undefined ? undefined : JSON.stringify(data),
      signal: AbortSignal.timeout(30_000),
    });
    if (response.status === 404 && allowMissing) return undefined;
    if (!response.ok) throw new Error(`GitHub ${method} ${resource} failed (${response.status}).`);
    return response.status === 204 ? undefined : response.json();
  }
  const files = [];
  for (const target of supportedTargets()) {
    const name = archiveName(sha, target);
    const content = await readFile(join(resolve("artifacts"), name));
    const checksum = await readFile(join(resolve("artifacts"), `${name}.sha256`));
    const expected = parseChecksumFile(checksum.toString("utf8"), name);
    if (createHash("sha256").update(content).digest("hex") !== expected) {
      throw new Error(`Refusing to publish ${name}: archive checksum does not match.`);
    }
    files.push({ name, content }, { name: `${name}.sha256`, content: checksum });
  }
  let release = await request(`releases/tags/${tag}`, "GET", undefined, true);
  if (release && !release.draft) {
    for (const target of supportedTargets()) {
      if (!releaseAssets(release, sha, target))
        throw new Error(`Published ${tag} has incomplete assets; refusing to replace it.`);
    }
    console.log(`${tag} is already published; retaining its existing assets.`);
    return;
  }
  if (!release) {
    release = await request("releases", "POST", {
      tag_name: tag,
      target_commitish: sha,
      name: `Rusteward main @ ${sha.slice(0, 12)}`,
      body: `Prebuilt cargo-dev binaries from main commit ${sha}.\n\nInstall with RhNu/Rusteward's installation Action. Each archive includes a source/platform manifest and has a SHA256 checksum.`,
      draft: true,
      prerelease: true,
      make_latest: "false",
    });
  }
  for (const file of files) {
    const previous = release.assets?.find((asset) => asset.name === file.name);
    if (previous) await request(`releases/assets/${previous.id}`, "DELETE");
    console.log(`Uploading ${file.name} to draft ${tag}.`);
    const response = await fetch(
      `https://uploads.github.com/repos/${repository}/releases/${release.id}/assets?name=${encodeURIComponent(file.name)}`,
      {
        method: "POST",
        headers: { ...headers, "Content-Type": "application/octet-stream" },
        body: file.content,
        signal: AbortSignal.timeout(120_000),
      },
    );
    if (!response.ok)
      throw new Error(
        `Upload of ${file.name} failed (${response.status}); ${tag} remains a draft.`,
      );
  }
  await request(`releases/${release.id}`, "PATCH", {
    draft: false,
    prerelease: true,
    make_latest: "false",
  });
  console.log(`Published ${tag} with all three platform packages and checksums.`);
}

await publish();
