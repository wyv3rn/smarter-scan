#!/usr/bin/env bash
scanimage --resolution 300 --source="ADF" --batch="${1}/${2}-%d.pnm"
