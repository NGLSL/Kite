/**
 * 在子进程中确保 Rust 工具链 PATH 可见（VS Code 旧终端可能缺 cargo）。
 * 用法: node scripts/run-tauri.mjs dev|build
 */
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const cargoBin = "D:\\Tools\\cargo\\bin";

const env = { ...process.env };
if (existsSync(cargoBin) && !env.PATH?.split(";").includes(cargoBin)) {
  env.PATH = `${cargoBin};${env.PATH ?? ""}`;
}
env.RUSTUP_HOME = env.RUSTUP_HOME || "D:\\Tools\\rustup";
env.CARGO_HOME = env.CARGO_HOME || "D:\\Tools\\cargo";

const sub = process.argv[2] === "build" ? "build" : "dev";
const command = process.platform === "win32" ? "cmd.exe" : "npx";
const args = process.platform === "win32"
  ? ["/d", "/s", "/c", `npx.cmd tauri ${sub}`]
  : ["tauri", sub];
const child = spawn(command, args, {
  cwd: root,
  env,
  stdio: "inherit",
});
child.on("exit", (code) => process.exit(code ?? 1));
