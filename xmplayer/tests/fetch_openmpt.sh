#!/bin/bash
set -e
mkdir -p xmplayer/test_data
cd xmplayer/test_data
echo "Downloading tests..."
curl -sLO https://raw.githubusercontent.com/OpenMPT/openmpt/master/test/test.s3m
curl -sLO https://raw.githubusercontent.com/OpenMPT/openmpt/master/test/test.xm
curl -sLO https://raw.githubusercontent.com/OpenMPT/openmpt/master/test/test.mod
curl -sLO https://raw.githubusercontent.com/OpenMPT/openmpt/master/test/test.it
echo "Done"
