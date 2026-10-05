import { build } from "esbuild";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const workspaceRoot = fileURLToPath(new URL("../../", import.meta.url));

/** Build the self-contained entry point shipped with the Action. */
export async function bundleAction() {
  const result = await build({
    absWorkingDir: workspaceRoot,
    entryPoints: ["action/src/install.mjs"],
    outfile: "action/dist/index.cjs",
    bundle: true,
    platform: "node",
    target: "node24",
    format: "cjs",
    minify: true,
    legalComments: "linked",
    metafile: true,
    write: false,
    logLevel: "warning",
  });
  // The distributed bundle carries the license texts of packages whose code it includes.
  const packages = new Set();
  for (const input of Object.keys(result.metafile.inputs)) {
    const parts = input.replaceAll("\\", "/").split("/");
    const index = parts.lastIndexOf("node_modules");
    if (index !== -1) {
      packages.add(parts.slice(0, index + (parts[index + 1].startsWith("@") ? 3 : 2)).join("/"));
    }
  }
  const licenses = [];
  for (const directory of [...packages].sort()) {
    const absolute = join(workspaceRoot, directory);
    const metadata = JSON.parse(await readFile(join(absolute, "package.json"), "utf8"));
    licenses.push(
      `${metadata.name}@${metadata.version} (${metadata.license || "see license text"})`,
    );
    for (const file of (await readdir(absolute))
      .filter((name) => /^licen[sc]e(?:[.-].*)?$/i.test(name))
      .sort()) {
      licenses.push(await readFile(join(absolute, file), "utf8"));
    }
  }
  result.outputFiles.push({
    path: join(workspaceRoot, "action/dist/LICENSES.txt"),
    contents: Buffer.from(`${licenses.join("\n\n").replace(/\r\n?/g, "\n").trimEnd()}\n`),
  });
  return result;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const result = await bundleAction();
  await mkdir(new URL("../dist/", import.meta.url), { recursive: true });
  for (const output of result.outputFiles) await writeFile(output.path, output.contents);
  console.log("Built action/dist/index.cjs and dependency license notices.");
}
