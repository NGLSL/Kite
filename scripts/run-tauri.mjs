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
// Windows 进程里环境变量名是 Path（PowerShell/CMD）或 PATH（Git Bash）。
// 必须找到真实那个键来改；否则会同时存在两个仅大小写不同的变量，
// 子进程环境块损坏，spawn cmd.exe 直接 ENOENT。
const pathKey = Object.keys(env).find((k) => k.toLowerCase() === "path");
if (pathKey) {
  const has = env[pathKey]
    .split(";")
    .some((p) => p.trim().toLowerCase() === cargoBin.toLowerCase());
  if (existsSync(cargoBin) && !has) {
    env[pathKey] = `${cargoBin};${env[pathKey]}`;
  }
} else if (existsSync(cargoBin)) {
  env.PATH = cargoBin;
}
env.RUSTUP_HOME = env.RUSTUP_HOME || "D:\\Tools\\rustup";
env.CARGO_HOME = env.CARGO_HOME || "D:\\Tools\\cargo";

const sub = process.argv[2] === "build" ? "build" : "dev";
// ComSpec 是 cmd.exe 的绝对路径，PATH 缺 System32 时也能找到
const command = process.platform === "win32" ? (process.env.ComSpec || "cmd.exe") : "npx";
const args = process.platform === "win32"
  ? ["/d", "/s", "/c", `npx.cmd tauri ${sub}`]
  : ["tauri", sub];
const child = spawn(command, args, {
  cwd: root,
  env,
  stdio: "inherit",
});
child.on("exit", (code) => process.exit(code ?? 1));
