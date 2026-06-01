/**
 * 从一个原始 PNG 生成 Tauri 全套图标。
 * - 所有平台基础图标：`pnpm exec tauri icon`
 * - macOS 细化图标：`gen-macos-icon.mjs`（仅在 darwin 执行）
 *
 * 用法：
 *   pnpm run gen:icons -- --input ../../docs/icon.png
 *   pnpm run gen:icons -- --input ../../docs/icon.png --icons-dir ../client/src-tauri/icons
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { spawnSync } from "child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_ICONS_DIR = path.resolve(__dirname, "../../client/src-tauri/icons");

function parseArgs(argv) {
  const args = { input: null, iconsDir: DEFAULT_ICONS_DIR };
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (token === "--input") {
      args.input = argv[i + 1] ?? null;
      i += 1;
      continue;
    }
    if (token === "--icons-dir") {
      args.iconsDir = argv[i + 1] ?? args.iconsDir;
      i += 1;
    }
  }
  return args;
}

function run(command, args, cwd, failedMessage) {
  const result = spawnSync(command, args, { cwd, stdio: "inherit", shell: false });
  if (result.status !== 0) {
    throw new Error(failedMessage);
  }
}

function tryRun(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, stdio: "inherit", shell: false });
  return result.status === 0;
}

async function main() {
  const { input, iconsDir } = parseArgs(process.argv.slice(2));
  if (!input) {
    console.error("请传入 --input <png 文件路径>");
    process.exit(1);
  }

  const inputPath = path.resolve(process.cwd(), input);
  const targetIconsDir = path.resolve(process.cwd(), iconsDir);
  if (!fs.existsSync(inputPath)) {
    console.error(`未找到输入图片: ${inputPath}`);
    process.exit(1);
  }
  fs.mkdirSync(targetIconsDir, { recursive: true });

  console.log("1/2 使用 tauri icon 生成基础图标...");
  const localOk = tryRun("pnpm", ["exec", "tauri", "icon", inputPath, "-o", targetIconsDir], process.cwd());
  if (!localOk) {
    console.log("未检测到可用的本地 tauri CLI，尝试使用 pnpm dlx 临时执行...");
    run(
      "pnpm",
      ["dlx", "@tauri-apps/cli", "tauri", "icon", inputPath, "-o", targetIconsDir],
      process.cwd(),
      "tauri icon 执行失败，请检查网络或手动安装 @tauri-apps/cli。"
    );
  }

  if (process.platform !== "darwin") {
    console.log("2/2 当前非 macOS，跳过 macOS 专用 iconutil 细化步骤。");
    console.log(`完成，已写入: ${targetIconsDir}`);
    return;
  }

  console.log("2/2 在 macOS 上细化 icon.icns 与关键 PNG 尺寸...");
  const macScript = path.join(__dirname, "gen-macos-icon.mjs");
  run(
    process.execPath,
    [macScript, "--input", inputPath, "--icons-dir", targetIconsDir],
    process.cwd(),
    "gen-macos-icon 执行失败。"
  );

  console.log(`完成，已写入: ${targetIconsDir}`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
