#!/bin/bash

if [ $# -ne 1 ]; then
    echo "Usage: ./deploy.sh host"
    exit -1
fi

host=$1
src=target/aarch64-unknown-linux-musl/release/smarter-scan

cargo build --target=aarch64-unknown-linux-musl --release &&
ssh $host systemctl stop smarter-scan || true
scp $src $host:/usr/local/bin/smarter-scan
scp scripts/scan.sh $host:/usr/local/bin/smarter-scan-do
scp scripts/post-processing.sh $host:/usr/local/bin/smarter-scan-post
scp assets/smarter-scan.service $host:/lib/systemd/system/smarter-scan.service
ssh $host systemctl daemon-reload || true
ssh $host systemctl restart smarter-scan
