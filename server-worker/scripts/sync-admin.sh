#!/bin/sh
# 控制台单一来源在 server/src/admin.html；这里只复制，不手改
set -e
cd "$(dirname "$0")/.."
mkdir -p public
cp ../server/src/admin.html public/admin.html
cp ../server/src/admin.html public/index.html
