#!/usr/bin/env bash

source .env

case $1 in
  b)
    cargo b;;
  flash)
    cargo espflash flash --release --partition-table partition.csv --monitor -s 2mb;;
  *)
    echo "Unknown subcommand"
esac

