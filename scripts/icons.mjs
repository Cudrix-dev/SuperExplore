import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const output = mkdtempSync(join(tmpdir(), "superexplore-icons-"));
try {
  execFileSync(process.execPath, [
    join(root, "node_modules/@tauri-apps/cli/tauri.js"),
    "icon",
    join(root, "public/superexplore.svg"),
    "-o",
    output,
  ], { cwd: root, stdio: "inherit" });
  const icons = join(root, "src-tauri/icons");
  mkdirSync(icons, { recursive: true });
  copyFileSync(join(output, "icon.ico"), join(icons, "icon.ico"));
} finally {
  rmSync(output, { recursive: true, force: true });
}
