#!/usr/bin/env node
const { spawnSync } = require('node:child_process');
const path = require('node:path');
const fs = require('node:fs');

const binaryName = process.platform === 'win32' ? 'thanks-stars.exe' : 'thanks-stars';
const binaryPath = path.join(__dirname, '..', 'npm', 'bin', binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error('The thanks-stars binary could not be found.');
  console.error('Please reinstall the package or report an issue at https://github.com/Kenzo-Wada/thanks-stars.');
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: 'inherit',
});

if (result.error) {
  console.error(result.error);
  process.exit(1);
}

process.exit(result.status ?? 0);
