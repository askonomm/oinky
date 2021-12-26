#!/usr/bin/env bash

if [ "$(uname)" == "Darwin" ]; then
  curl -O -L https://github.com/askonomm/oinky/releases/latest/download/oinky-macos && \
  mv oinky-macos oinky && \
  chmod +x oinky
else
  curl -O -L https://github.com/askonomm/oinky/releases/latest/download/oinky-linux && \
  mv oinky-linux oinky && \
  chmod +x oinky
fi

while [[ "$#" -gt 0 ]]; do
  case $1 in
    -g|--global) global="true"; shift ;;
  esac
  shift
done

if [ "$global" == "true" ]; then
  sudo mv oinky /usr/local/bin/oinky
fi