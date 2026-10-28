#!/usr/bin/env bash
set -euo pipefail

# NFR-PRF-1 verification tooling: idle/ambient capture should average <=5% CPU and
# stay under 500 MB RSS (excluding any resident local LLM — this only samples the app
# process itself, not Ollama). Run this while Whimbrel is running with Capture ON
# and no active transcription/chat/digest call in progress (NFR-PRF-2 explicitly
# permits CPU bursts during those — this script measures the idle/ambient baseline).
#
# This only samples and reports; it doesn't touch the app's config or state, and it's
# safe to run against any process name for a quick sanity check of the tool itself.

APP_NAME="${1:-Whimbrel}"
DURATION_SECONDS="${2:-300}"
INTERVAL_SECONDS=2

pid=$(pgrep -x "$APP_NAME" | head -1 || true)
if [ -z "$pid" ]; then
  echo "error: no running process named '$APP_NAME' found" >&2
  echo "usage: $0 [process-name] [duration-seconds]" >&2
  exit 1
fi

echo "Sampling PID $pid ('$APP_NAME') every ${INTERVAL_SECONDS}s for ${DURATION_SECONDS}s..."

samples=0
cpu_sum="0"
rss_max_kb=0
end=$(( $(date +%s) + DURATION_SECONDS ))

while [ "$(date +%s)" -lt "$end" ]; do
  line=$(ps -o %cpu=,rss= -p "$pid" 2>/dev/null || true)
  if [ -z "$line" ]; then
    echo "process exited before sampling finished" >&2
    break
  fi
  cpu=$(echo "$line" | awk '{print $1}')
  rss_kb=$(echo "$line" | awk '{print $2}')
  cpu_sum=$(echo "$cpu_sum + $cpu" | bc)
  samples=$((samples + 1))
  if [ "$rss_kb" -gt "$rss_max_kb" ]; then
    rss_max_kb="$rss_kb"
  fi
  sleep "$INTERVAL_SECONDS"
done

if [ "$samples" -eq 0 ]; then
  echo "error: no samples collected (process may have exited immediately)" >&2
  exit 1
fi

avg_cpu=$(echo "scale=2; $cpu_sum / $samples" | bc)
rss_max_mb=$(( rss_max_kb / 1024 ))

echo ""
echo "Samples: $samples over ~$(( samples * INTERVAL_SECONDS ))s"
echo "Average CPU: ${avg_cpu}%  (budget: <=5%, NFR-PRF-1)"
echo "Peak RSS: ${rss_max_mb} MB  (budget: <=500 MB excluding any resident local LLM, NFR-PRF-1)"
echo ""

pass=true
if (( $(echo "$avg_cpu > 5" | bc -l) )); then
  echo "FAIL: average CPU exceeds the 5% budget"
  pass=false
fi
if [ "$rss_max_mb" -gt 500 ]; then
  echo "FAIL: peak RSS exceeds the 500 MB budget"
  pass=false
fi
if [ "$pass" = true ]; then
  echo "PASS: within NFR-PRF-1 budget"
fi
