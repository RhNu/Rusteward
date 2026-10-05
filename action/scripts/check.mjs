import { readFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { parseDocument } from "yaml";
import { bundleAction, workspaceRoot } from "./build.mjs";

/** Check source syntax, YAML syntax, and the generated entry point without rewriting files. */
async function checkAction() {
  for (const directory of ["action/src", "action/scripts", "action/tests"]) {
    for (const file of await readdir(join(workspaceRoot, directory))) {
      if (file.endsWith(".mjs")) {
        execFileSync(process.execPath, ["--check", join(workspaceRoot, directory, file)], {
          stdio: "inherit",
          windowsHide: true,
        });
      }
    }
  }
  for (const file of ["action.yml", ".github/workflows/ci.yml"]) {
    const document = parseDocument(await readFile(join(workspaceRoot, file), "utf8"), {
      uniqueKeys: true,
    });
    if (document.errors.length) throw new Error(`${file}: ${document.errors.join("; ")}`);
  }
  for (const output of (await bundleAction()).outputFiles) {
    const existing = await readFile(output.path);
    if (!existing.equals(Buffer.from(output.contents))) {
      throw new Error("The Action bundle is outdated. Run npm run build:action.");
    }
  }
  console.log("Action source syntax, YAML syntax, and generated bundle checks passed.");
}

await checkAction();
