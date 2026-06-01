/**
 * 从一个现有 PNG 生成 macOS 图标资源：
 * - 使用 ImageMagick 做缩放、圆角和留白
 * - 使用 macOS 自带 iconutil 产出 icon.icns
 *
 * 用法：
 *   pnpm run gen:macos-icon -- --input ../../docs/icon.png
 *   pnpm run gen:macos-icon -- --input ../../docs/icon.png --icons-dir ../client/src-tauri/icons
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { spawnSync } from "child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_ICONS_DIR = path.resolve(__dirname, "../../client/src-tauri/icons");

const CANVAS = 1024;
const PADDING = 100;
const CONTENT = CANVAS - PADDING * 2;
const RADIUS_824 = Math.round((185 * CONTENT) / CANVAS);

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

function getImageMagickCmd() {
  const magick = spawnSync("magick", ["-version"], { encoding: "utf8", stdio: "pipe" });
  if (magick.status === 0) return "magick";
  const convert = spawnSync("convert", ["-version"], { encoding: "utf8", stdio: "pipe" });
  return convert.status === 0 ? "convert" : null;
}

function runConvert(command, args, cwd) {
  const argv = args;
  const result = spawnSync(command, argv, { stdio: "inherit", cwd, shell: false });
  if (result.status !== 0) {
    throw new Error("ImageMagick 执行失败");
  }
}

function runIconutil(iconsDir, iconsetDir) {
  const test = spawnSync("which", ["iconutil"], { encoding: "utf8", stdio: "pipe" });
  if (test.status !== 0) {
    console.warn("未检测到 iconutil，跳过 icon.icns 精修（保留 tauri icon 生成结果）。");
    return false;
  }
  const result = spawnSync("iconutil", ["-c", "icns", "-o", path.join(iconsDir, "icon.icns"), iconsetDir], {
    stdio: "inherit",
    cwd: iconsDir,
  });
  if (result.status !== 0) {
    throw new Error("iconutil 执行失败");
  }
  return true;
}

async function main() {
  if (process.platform !== "darwin") {
    console.error("gen:macos-icon 仅支持在 macOS 上运行。");
    process.exit(1);
  }

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

  const command = getImageMagickCmd();
  if (!command) {
    console.error("未检测到 ImageMagick，请先安装。例如：brew install imagemagick");
    process.exit(1);
  }

  fs.mkdirSync(targetIconsDir, { recursive: true });

  const tmpContent = path.join(targetIconsDir, ".icon_824.png");
  const tmpMask = path.join(targetIconsDir, ".icon_mask.png");
  const tmpRounded = path.join(targetIconsDir, ".icon_824_rounded.png");
  const icon1024 = path.join(targetIconsDir, ".icon_1024.png");
  const iconsetDir = path.join(targetIconsDir, "AppIcon.iconset");

  try {
    console.log("1/7 缩放到 824x824...");
    runConvert(command, [inputPath, "-resize", `${CONTENT}x${CONTENT}`, path.basename(tmpContent)], targetIconsDir);

    console.log("2/7 生成圆角蒙版...");
    runConvert(
      command,
      [
        "-size",
        `${CONTENT}x${CONTENT}`,
        "xc:black",
        "-fill",
        "white",
        "-draw",
        `roundrectangle 0,0 ${CONTENT - 1},${CONTENT - 1} ${RADIUS_824},${RADIUS_824}`,
        path.basename(tmpMask),
      ],
      targetIconsDir
    );

    console.log("3/7 应用圆角...");
    runConvert(
      command,
      [path.basename(tmpContent), path.basename(tmpMask), "-compose", "CopyOpacity", "-composite", "-alpha", "set", path.basename(tmpRounded)],
      targetIconsDir
    );

    console.log("4/7 加透明留白到 1024x1024...");
    runConvert(
      command,
      [path.basename(tmpRounded), "-background", "none", "-gravity", "center", "-extent", `${CANVAS}x${CANVAS}`, path.basename(icon1024)],
      targetIconsDir
    );

    fs.mkdirSync(iconsetDir, { recursive: true });
    const sizes = [
      [16, "icon_16x16.png"],
      [32, "icon_16x16@2x.png"],
      [32, "icon_32x32.png"],
      [64, "icon_32x32@2x.png"],
      [128, "icon_128x128.png"],
      [256, "icon_128x128@2x.png"],
      [256, "icon_256x256.png"],
      [512, "icon_256x256@2x.png"],
      [512, "icon_512x512.png"],
      [1024, "icon_512x512@2x.png"],
    ];

    console.log("5/7 生成 iconset...");
    for (const [size, name] of sizes) {
      runConvert(command, [path.basename(icon1024), "-resize", `${size}x${size}`, `AppIcon.iconset/${name}`], targetIconsDir);
    }

    console.log("6/7 生成 icon.icns...");
    const iconutilOk = runIconutil(targetIconsDir, iconsetDir);
    if (!iconutilOk) {
      console.log("继续执行 PNG 导出步骤。");
    }

    console.log("7/7 导出 Tauri 常用 PNG 尺寸...");
    // 覆盖 icon.png，避免某些场景仍读取 tauri icon 生成的方形版本。
    runConvert(command, [path.basename(icon1024), "icon.png"], targetIconsDir);
    runConvert(command, [path.basename(icon1024), "-resize", "32x32", "32x32.png"], targetIconsDir);
    runConvert(command, [path.basename(icon1024), "-resize", "128x128", "128x128.png"], targetIconsDir);
    runConvert(command, [path.basename(icon1024), "-resize", "256x256", "128x128@2x.png"], targetIconsDir);

    console.log(`完成，已写入: ${targetIconsDir}`);
  } finally {
    for (const temp of [tmpContent, tmpMask, tmpRounded, icon1024]) {
      if (fs.existsSync(temp)) fs.unlinkSync(temp);
    }
    if (fs.existsSync(iconsetDir)) {
      for (const file of fs.readdirSync(iconsetDir)) {
        fs.unlinkSync(path.join(iconsetDir, file));
      }
      fs.rmdirSync(iconsetDir);
    }
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
