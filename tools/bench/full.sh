#!/bin/bash
# Whole benchmark: startup/idle rounds, then click tests for each client.
S="$(dirname "$0")"
cd "$S"
python -I bench.py stop steam; python -I bench.py stop ours
python -I run_all.py 5
for app in steam ours; do
  python -I bench.py startup $app --keep --tag nav-setup
  sleep 20
  python -I nav.py $app 5
  python -I bench.py stop $app
done
echo BENCHDONE
python -I record.py race
python -I record.py idle
echo ALLDONE
