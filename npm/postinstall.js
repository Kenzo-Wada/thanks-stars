#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const https = require('node:https');
const { pipeline } = require('node:stream/promises');
const tar = require('tar');
const extractZip = require('extract-zip');

const pkg = require('../package.json');

const SUPPORTED_TARGETS = {
  'darwin-x64': { triple: 'x86_64-apple-darwin', ext: 'tar.gz' },
  'darwin-arm64': { triple: 'aarch64-apple-darwin', ext: 'tar.gz' },
  'linux-x64': { triple: 'x86_64-unknown-linux-gnu', ext: 'tar.gz' },
  'win32-x64': { triple: 'x86_64-pc-windows-msvc', ext: 'zip' }
};

function resolveTarget() {
  if (process.env.THANKS_STARS_NPM_TARGET) {
    const value = process.env.THANKS_STARS_NPM_TARGET.trim();
    if (!value) {
      return null;
    }
    const [triple, ext] = value.split(':');
    if (!triple || !ext) {
      throw new Error(`Invalid THANKS_STARS_NPM_TARGET format: ${value}. Expected <target-triple>:<ext>`);
    }
    return { triple, ext };
  }

  const key = `${process.platform}-${process.arch}`;
  return SUPPORTED_TARGETS[key] ?? null;
}

function download(url, destination) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, (response) => {
      if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
        const next = new URL(response.headers.location, url).toString();
        response.resume();
        resolve(download(next, destination));
        return;
      }

      if (response.statusCode !== 200) {
        reject(new Error(`Failed to download ${url} (status: ${response.statusCode})`));
        response.resume();
        return;
      }

      const file = fs.createWriteStream(destination, { mode: 0o755 });
      pipeline(response, file).then(resolve).catch(reject);
    });

    request.on('error', reject);
  });
}

async function extractArchive(archivePath, destinationDir, ext) {
  await fs.promises.rm(destinationDir, { recursive: true, force: true });
  await fs.promises.mkdir(destinationDir, { recursive: true });

  if (ext === 'zip') {
    await extractZip(archivePath, { dir: destinationDir });
  } else if (ext === 'tar.gz') {
    await tar.x({ file: archivePath, cwd: destinationDir, strip: 1 });
  } else {
    throw new Error(`Unsupported archive extension: ${ext}`);
  }
}

async function ensureBinaryAtRoot(binDir, binaryName) {
  const directPath = path.join(binDir, binaryName);
  if (await exists(directPath)) {
    return directPath;
  }

  const entries = await fs.promises.readdir(binDir, { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isDirectory()) {
      continue;
    }
    const candidate = path.join(binDir, entry.name, binaryName);
    if (await exists(candidate)) {
      try {
        await fs.promises.rename(candidate, directPath);
      } catch (error) {
        if (error.code === 'EXDEV') {
          await fs.promises.copyFile(candidate, directPath);
          await fs.promises.unlink(candidate);
        } else {
          throw error;
        }
      }
      return directPath;
    }
  }

  return directPath;
}

async function exists(filePath) {
  try {
    await fs.promises.access(filePath, fs.constants.F_OK);
    return true;
  } catch {
    return false;
  }
}

async function main() {
  const target = resolveTarget();
  if (!target) {
    console.error('Failed to download a thanks-stars binary for this platform.');
    console.error(`Platform ${process.platform} (${process.arch}) is not currently supported.`);
    console.error('You can build from source with Cargo: https://crates.io/crates/thanks-stars');
    process.exitCode = 1;
    return;
  }

  const version = pkg.version;
  const tag = `v${version}`;
  const assetNameCandidates = [
    `thanks-stars-${tag}-${target.triple}.${target.ext}`,
    `thanks-stars-${target.triple}.${target.ext}`
  ];

  const tempFile = path.join(os.tmpdir(), `thanks-stars-${target.triple}.${target.ext}`);
  try {
    let downloadedAsset = null;
    let lastError = null;
    for (const assetName of assetNameCandidates) {
      const url = `https://github.com/Kenzo-Wada/thanks-stars/releases/download/${tag}/${assetName}`;
      console.log(`Downloading ${url}`);
      try {
        await download(url, tempFile);
        downloadedAsset = assetName;
        break;
      } catch (error) {
        lastError = error;
        if (error?.message?.includes('(status: 404)')) {
          console.warn(`Asset not found: ${assetName}. Trying next candidate if available.`);
          continue;
        }
        throw error;
      }
    }

    if (!downloadedAsset) {
      throw lastError ?? new Error('Failed to download any release asset.');
    }

    const binDir = path.join(__dirname, 'bin');
    await extractArchive(tempFile, binDir, target.ext);

    const binaryName = process.platform === 'win32' ? 'thanks-stars.exe' : 'thanks-stars';
    const binaryPath = await ensureBinaryAtRoot(binDir, binaryName);
    if (!(await exists(binaryPath))) {
      throw new Error(`Binary not found after extraction: ${binaryPath}`);
    }

    if (process.platform !== 'win32') {
      await fs.promises.chmod(binaryPath, 0o755);
    }

    console.log('thanks-stars is ready to use!');
  } catch (error) {
    console.error('Failed to set up the thanks-stars binary.');
    console.error(error.message);
    process.exitCode = 1;
  } finally {
    fs.promises.rm(tempFile, { force: true }).catch(() => {});
  }
}

main();
