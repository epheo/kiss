#!/bin/bash

# Performance Comparison: KISS vs nginx
# Tests file sizes, concurrency levels, and caching performance

set -e

# Configuration
KISS_URL="http://localhost:8080"
NGINX_URL="http://localhost:80"
DURATION=10
THREADS=4
LOG_DIR="benchmark_results"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Create results directory
mkdir -p "$LOG_DIR"
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")

echo -e "${BLUE}=== KISS vs nginx Performance Benchmark ===${NC}"
echo "Timestamp: $(date)"
echo "Duration per test: ${DURATION}s"
echo "Threads: $THREADS"
echo "Results will be saved in: $LOG_DIR"
echo ""

# Function to run wrk test and extract key metrics
run_wrk_test() {
    local url="$1"
    local connections="$2"
    local file="$3"
    local headers="$4"
    
    local output_file="$LOG_DIR/wrk_${file}_c${connections}_$(basename $url | tr ':' '_').log"
    
    if [ -n "$headers" ]; then
        wrk -t$THREADS -c$connections -d${DURATION}s --latency -H "$headers" "$url/$file" > "$output_file" 2>&1
    else
        wrk -t$THREADS -c$connections -d${DURATION}s --latency "$url/$file" > "$output_file" 2>&1
    fi
    
    # Extract metrics - use head -1 to get first match for Latency line
    local rps=$(grep "^Requests/sec:" "$output_file" | awk '{print $2}')
    local avg_latency=$(grep "Thread Stats" "$output_file" -A 1 | grep "Latency" | awk '{print $2}')
    local transfer_rate=$(grep "^Transfer/sec:" "$output_file" | awk '{print $2}')
    local p50=$(grep "Latency Distribution" "$output_file" -A 10 | grep "50%" | awk '{print $2}')
    local p99=$(grep "Latency Distribution" "$output_file" -A 10 | grep "99%" | awk '{print $2}')
    
    # Handle socket errors if present
    local errors=$(grep "Socket errors:" "$output_file" | sed 's/.*connect//' | awk '{gsub(/[^0-9 ]/,"")} {sum=0; for(i=1;i<=NF;i++) sum+=$i} END {print sum}')
    [ -z "$errors" ] && errors=0
    
    # Set defaults for missing values
    [ -z "$rps" ] && rps=0
    [ -z "$avg_latency" ] && avg_latency="N/A"
    [ -z "$transfer_rate" ] && transfer_rate="N/A"
    [ -z "$p50" ] && p50="N/A"
    [ -z "$p99" ] && p99="N/A"
    
    echo "$rps,$avg_latency,$transfer_rate,$p50,$p99,$errors"
}

# Function to print test results table
print_results_table() {
    local test_name="$1"
    local file="$2"
    local connections="$3"
    local headers="$4"
    
    echo -e "\n${YELLOW}=== $test_name ===${NC}"
    printf "%-10s %-12s %-12s %-15s %-8s %-8s %-8s\n" "Server" "RPS" "Latency" "Transfer" "P50" "P99" "Errors"
    printf "%-10s %-12s %-12s %-15s %-8s %-8s %-8s\n" "------" "---" "-------" "--------" "---" "---" "------"
    
    # Test KISS
    echo -n "KISS       "
    kiss_results=$(run_wrk_test "$KISS_URL" "$connections" "$file" "$headers")
    echo "$kiss_results" | tr ',' '\t' | awk '{printf "%-12s %-12s %-15s %-8s %-8s %-8s\n", $1, $2, $3, $4, $5, $6}'
    
    # Test nginx
    echo -n "nginx      "
    nginx_results=$(run_wrk_test "$NGINX_URL" "$connections" "$file" "$headers")
    echo "$nginx_results" | tr ',' '\t' | awk '{printf "%-12s %-12s %-15s %-8s %-8s %-8s\n", $1, $2, $3, $4, $5, $6}'
    
    # Calculate ratio
    kiss_rps=$(echo "$kiss_results" | cut -d',' -f1)
    nginx_rps=$(echo "$nginx_results" | cut -d',' -f1)
    
    # Only calculate ratio if both values are numeric and non-zero
    if [[ "$kiss_rps" =~ ^[0-9]+(\.[0-9]+)?$ ]] && [[ "$nginx_rps" =~ ^[0-9]+(\.[0-9]+)?$ ]] && [ "$kiss_rps" != "0" ] && [ "$nginx_rps" != "0" ]; then
        ratio=$(echo "scale=2; $kiss_rps / $nginx_rps" | bc -l 2>/dev/null)
        if [ $? -eq 0 ]; then
            # Compare and show which is faster
            if (( $(echo "$kiss_rps > $nginx_rps" | bc -l) )); then
                echo -e "${GREEN}KISS is ${ratio}x faster${NC}"
            else
                ratio=$(echo "scale=2; $nginx_rps / $kiss_rps" | bc -l)
                echo -e "${GREEN}nginx is ${ratio}x faster${NC}"
            fi
        fi
    fi
}

# Test 1: File Size Performance
echo -e "${BLUE}Testing file size performance...${NC}"

print_results_table "Small File (index.html)" "index.html" "100" ""
print_results_table "Medium File (medium.txt)" "medium.txt" "100" ""
print_results_table "Large File (large.txt)" "large.txt" "100" ""

# Test 2: Concurrency Scaling
echo -e "\n${BLUE}Testing concurrency scaling with index.html...${NC}"

for connections in 1 10 50 100 200 500; do
    print_results_table "Connections $connections" "index.html" "$connections" ""
done

# Test 3: Cache Performance (304 responses)
echo -e "\n${BLUE}Testing cache performance (304 Not Modified)...${NC}"

future_date="Wed, 21 Oct 2025 07:28:00 GMT"
print_results_table "Cache Test (304)" "index.html" "100" "If-Modified-Since: $future_date"

# Test 4: Health Endpoint Performance
echo -e "\n${BLUE}Testing health endpoint performance...${NC}"

print_results_table "Health Endpoint" "health" "100" ""

# Summary
echo -e "\n${BLUE}=== Performance Summary ===${NC}"
echo "Benchmark completed at: $(date)"
echo "Detailed logs saved in: $LOG_DIR"
echo ""
echo -e "${GREEN}Key Findings:${NC}"
echo "- Check the ratios above to see relative performance"
echo "- Look for socket errors (should be 0)"
echo "- Compare P99 latencies for tail performance"
echo "- Review logs in $LOG_DIR for detailed analysis"

echo -e "\n${YELLOW}Tip: Run 'ls -la $LOG_DIR' to see all generated log files${NC}"