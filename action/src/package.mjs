import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { archiveName, platformFor, validateManifest, validateSha } from "./model.mjs";

/** Package the built executable with source identity and independent archive/binary checksums. */
async function packageBinary() {
  const sha = validateSha(process.env.GITHUB_SHA);
  const platform = platformFor(process.env.RUNNER_OS, process.env.RUNNER_ARCH);
  const directory = resolve("artifacts");
  const archive = archiveName(sha, platform.target);
  const binaryPath = resolve("target", platform.target, "release", platform.binary);
  const bytes = await readFile(binaryPath);
  const manifest = validateManifest(
    {
      schema_version: 1,
      source_sha: sha,
      target: platform.target,
      binary: platform.binary,
      binary_sha256: createHash("sha256").update(bytes).digest("hex"),
    },
    sha,
    platform.target,
  );
  const stage = await mkdtemp(join(tmpdir(), "rusteward-package-"));
  try {
    await mkdir(directory, { recursive: true });
    await copyFile(binaryPath, join(stage, platform.binary));
    await writeFile(join(stage, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
    const destination = join(directory, archive);
    if (platform.archiveExtension === "zip") {
      execFileSync(
        "pwsh",
        [
          "-NoLogo",
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          "$ErrorActionPreference = 'Stop'; Compress-Archive -LiteralPath (Join-Path $env:RUSTEWARD_PACKAGE_STAGE 'cargo-dev.exe'), (Join-Path $env:RUSTEWARD_PACKAGE_STAGE 'manifest.json') -DestinationPath $env:RUSTEWARD_PACKAGE_OUTPUT -Force",
        ],
        {
          stdio: "inherit",
          windowsHide: true,
          env: {
            ...process.env,
            RUSTEWARD_PACKAGE_STAGE: stage,
            RUSTEWARD_PACKAGE_OUTPUT: destination,
          },
        },
      );
    } else {
      execFileSync("tar", ["-czf", destination, "-C", stage, platform.binary, "manifest.json"], {
        stdio: "inherit",
      });
    }
    const hash = createHash("sha256")
      .update(await readFile(destination))
      .digest("hex");
    await writeFile(`${destination}.sha256`, `${hash}  ${archive}\n`);
    console.log(`Packaged ${archive} (SHA256 ${hash}).`);
  } finally {
    await rm(stage, { recursive: true, force: true });
  }
}

await packageBinary();
