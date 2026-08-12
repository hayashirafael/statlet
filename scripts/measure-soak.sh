#!/bin/bash

set -euo pipefail

app_input="${1:?usage: measure-soak.sh /path/to/Statlet.app [duration_seconds] [interval_seconds] [output_dir]}"
app="$(cd "$(dirname "$app_input")" && pwd)/$(basename "$app_input")"
duration="${2:-1800}"
interval="${3:-10}"
output_dir="${4:-$(pwd)/dist/soak}"
executable="$app/Contents/MacOS/Statlet"
scenario="${STATLET_SOAK_SCENARIO:-Idle menu-bar sampling; Mole integration disabled; no UI interaction during samples}"

if [[ -z "$output_dir" || "$output_dir" == "/" ]]; then
    echo "Refusing unsafe soak output directory: '$output_dir'" >&2
    exit 1
fi

if [[ ! -x "$executable" ]]; then
    echo "Missing Statlet executable at $executable" >&2
    exit 1
fi
if ! [[ "$duration" =~ ^[0-9]+$ && "$interval" =~ ^[0-9]+$ && "$duration" -ge "$interval" && "$interval" -gt 0 ]]; then
    echo "Duration and interval must be positive integers, with duration >= interval" >&2
    exit 1
fi
find_exact_process() {
    local candidate command
    while IFS= read -r candidate; do
        command="$(ps -p "$candidate" -o command= 2>/dev/null || true)"
        if [[ "$command" == "$executable" ]]; then
            echo "$candidate"
            return 0
        fi
    done < <(pgrep -x Statlet || true)
    return 1
}

if find_exact_process >/dev/null; then
    echo "A process from this bundle is already running; refusing to mix measurements" >&2
    exit 1
fi

mkdir -p "$output_dir"
csv="$output_dir/samples.csv"
report="$output_dir/report.md"
footprint_start="$output_dir/footprint-start.txt"
footprint_end="$output_dir/footprint-end.txt"
warmup_seconds=10
rm -f "$report" "$footprint_end"

bundle_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
hardware_model="$(sysctl -n hw.model)"
processor="$(sysctl -n machdep.cpu.brand_string)"
os_version="$(sw_vers -productVersion)"
os_build="$(sw_vers -buildVersion)"
architecture="$(uname -m)"
executable_sha256="$(shasum -a 256 "$executable" | awk '{ print $1 }')"
signature_info="$(codesign --display --verbose=4 "$app" 2>&1)"
signature="$(awk -F= '/^Signature=/ { print $2; exit }' <<<"$signature_info")"
signature_flags="$(sed -n 's/^CodeDirectory .* flags=\([^ ]*\).*/flags=\1/p' <<<"$signature_info")"

open -n "$app"
pid=""
for _ in {1..50}; do
    pid="$(find_exact_process || true)"
    [[ -n "$pid" ]] && break
    sleep 0.1
done
if [[ -z "$pid" ]]; then
    echo "Statlet did not start" >&2
    exit 1
fi

cleanup() {
    if kill -0 "$pid" 2>/dev/null; then
        kill -TERM "$pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

sleep "$warmup_seconds"
footprint --pid "$pid" --noCategories --format bytes >"$footprint_start"
perl -0pi -e 's/\n+\z/\n/' "$footprint_start"
echo "timestamp_unix,elapsed_seconds,rss_kib,cpu_percent,cpu_time,context_switches,idle_wakeups" >"$csv"

started="$(date +%s)"
started_utc="$(date -u +'%Y-%m-%dT%H:%M:%SZ')"
deadline=$((started + duration))
while :; do
    now="$(date +%s)"
    elapsed=$((now - started))
    read -r rss cpu cpu_time < <(ps -p "$pid" -o rss=,%cpu=,time=)
    read -r context_switches idle_wakeups < <(
        top -l 1 -pid "$pid" -stats pid,csw,idlew | awk -v pid="$pid" '$1 == pid { print $2, $3 }'
    )
    context_switches="${context_switches//+/}"
    idle_wakeups="${idle_wakeups//+/}"
    echo "$now,$elapsed,$rss,$cpu,$cpu_time,$context_switches,$idle_wakeups" >>"$csv"
    [[ "$now" -ge "$deadline" ]] && break
    sleep "$interval"
done

footprint --pid "$pid" --noCategories --format bytes >"$footprint_end"
perl -0pi -e 's/\n+\z/\n/' "$footprint_end"

read -r samples elapsed_last rss_min rss_max rss_first rss_last cpu_average context_first context_last idle_first idle_last < <(
    awk -F, '
        NR == 2 {
            count = 1; min = max = first = last = $3; cpu = $4;
            cf = cl = $6; idle_first = idle_last = $7; next
        }
        NR > 2 {
            count++; if ($3 < min) min = $3; if ($3 > max) max = $3;
            last = $3; cpu += $4; cl = $6; idle_last = $7
        }
        END { print count, $2, min, max, first, last, cpu / count, cf, cl, idle_first, idle_last }
    ' "$csv"
)

rss_range_kib=$((rss_max - rss_min))
rss_growth_kib=$((rss_last - rss_first))
rss_peak_growth_kib=$((rss_max - rss_first))
context_switch_delta=$((context_last - context_first))
context_switches_per_second="$(awk -v total="$context_switch_delta" -v seconds="$elapsed_last" 'BEGIN { printf "%.2f", total / seconds }')"
idle_wakeup_delta=$((idle_last - idle_first))
idle_wakeups_per_second="$(awk -v total="$idle_wakeup_delta" -v seconds="$elapsed_last" 'BEGIN { printf "%.3f", total / seconds }')"
physical_start="$(awk '/^[[:space:]]+phys_footprint:/ { print $2; exit }' "$footprint_start")"
physical_end="$(awk '/^[[:space:]]+phys_footprint:/ { print $2; exit }' "$footprint_end")"
physical_peak="$(awk '/^[[:space:]]+phys_footprint_peak:/ { print $2; exit }' "$footprint_end")"

{
    echo "# Statlet v1 production-bundle soak"
    echo
    echo "- Started: ${started_utc}"
    echo "- Bundle: Statlet ${bundle_version}; executable SHA-256 \`${executable_sha256}\`"
    echo "- Signature: ${signature}; ${signature_flags}"
    echo "- Host: ${hardware_model}, ${processor}, ${architecture}, macOS ${os_version} (${os_build})"
    echo "- Scenario: ${scenario}"
    echo "- Requested duration: ${duration} seconds; observed duration: ${elapsed_last} seconds"
    echo "- Warm-up excluded from samples: ${warmup_seconds} seconds"
    echo "- Requested sampling interval: ${interval} seconds"
    echo "- Samples: ${samples}"
    echo "- RSS: ${rss_first} KiB initial, ${rss_last} KiB final, ${rss_min}–${rss_max} KiB observed"
    echo "- RSS growth: ${rss_growth_kib} KiB; peak above initial: ${rss_peak_growth_kib} KiB; observed range: ${rss_range_kib} KiB"
    echo "- Physical footprint: ${physical_start} bytes initial, ${physical_end} bytes final, ${physical_peak} bytes end-snapshot peak"
    echo "- Mean sampled CPU: ${cpu_average}%"
    echo "- Idle-wakeup delta: ${idle_wakeup_delta} (${idle_wakeups_per_second}/s; macOS top IDLEW counter)"
    echo "- Context-switch delta: ${context_switch_delta} (${context_switches_per_second}/s; secondary scheduling context)"
    echo "- Detailed physical-footprint snapshots: footprint-start.txt and footprint-end.txt"
    echo "- Raw samples: samples.csv"
    echo
    echo "The run passes the v1 bounded-growth guard when final RSS growth stays below 10 MiB, peak RSS stays below 20 MiB above the first post-warm-up sample, and mean sampled CPU stays below 1%. The full observed range remains informational because releasing memory increases that range without indicating a leak."
} >"$report"

if ((rss_growth_kib > 10240 || rss_peak_growth_kib > 20480)); then
    echo "Soak failed the bounded-memory guard; see $report" >&2
    exit 1
fi
if ! awk -v cpu="$cpu_average" 'BEGIN { exit !(cpu < 1.0) }'; then
    echo "Soak failed the CPU guard; see $report" >&2
    exit 1
fi

echo "Soak passed: $report"
