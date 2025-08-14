#!/bin/bash

# Performance Comparison: KISS vs nginx
# Tests file sizes, concurrency levels, and caching performance

set -e

# Configuration
KISS_URL="http://localhost:8080"
NGINX_URL="http://localhost:80"
REQUESTS=10000
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
echo "Requests per test: $REQUESTS"
echo "Results will be saved in: $LOG_DIR"
echo ""

# Function to run ab test and extract key metrics
run_ab_test() {
    local url="$1"
    local concurrency="$2"
    local file="$3"
    local headers="$4"
    
    local output_file="$LOG_DIR/ab_${file}_c${concurrency}_$(basename $url | tr ':' '_').log"
    
    if [ -n "$headers" ]; then
        ab -n $REQUESTS -c $concurrency -k -H "$headers" "$url/$file" > "$output_file" 2>&1
    else
        ab -n $REQUESTS -c $concurrency -k "$url/$file" > "$output_file" 2>&1
    fi
    
    # Extract metrics
    local rps=$(grep "Requests per second:" "$output_file" | awk '{print $4}')
    local time_per_req=$(grep "Time per request:" "$output_file" | head -1 | awk '{print $4}')
    local transfer_rate=$(grep "Transfer rate:" "$output_file" | awk '{print $3}')
    local p50=$(grep "50%" "$output_file" | awk '{print $2}')
    local p99=$(grep "99%" "$output_file" | awk '{print $2}')
    local failed=$(grep "Failed requests:" "$output_file" | awk '{print $3}')
    
    echo "$rps,$time_per_req,$transfer_rate,$p50,$p99,$failed"
}

# Function to print test results table
print_results_table() {
    local test_name="$1"
    local file="$2"
    local concurrency="$3"
    local headers="$4"
    
    echo -e "\n${YELLOW}=== $test_name ===${NC}"
    printf "%-10s %-12s %-12s %-15s %-8s %-8s %-8s\n" "Server" "RPS" "Time(ms)" "Transfer(KB/s)" "P50(ms)" "P99(ms)" "Failed"
    printf "%-10s %-12s %-12s %-15s %-8s %-8s %-8s\n" "------" "---" "-------" "-------------" "------" "------" "------"
    
    # Test KISS
    echo -n "KISS       "
    kiss_results=$(run_ab_test "$KISS_URL" "$concurrency" "$file" "$headers")
    echo "$kiss_results" | tr ',' '\t' | awk '{printf "%-12s %-12s %-15s %-8s %-8s %-8s\n", $1, $2, $3, $4, $5, $6}'
    
    # Test nginx
    echo -n "nginx      "
    nginx_results=$(run_ab_test "$NGINX_URL" "$concurrency" "$file" "$headers")
    echo "$nginx_results" | tr ',' '\t' | awk '{printf "%-12s %-12s %-15s %-8s %-8s %-8s\n", $1, $2, $3, $4, $5, $6}'
    
    # Calculate ratio
    kiss_rps=$(echo "$kiss_results" | cut -d',' -f1)
    nginx_rps=$(echo "$nginx_results" | cut -d',' -f1)
    
    if [ -n "$kiss_rps" ] && [ -n "$nginx_rps" ] && [ "$kiss_rps" != "0" ]; then
        ratio=$(echo "scale=2; $nginx_rps / $kiss_rps" | bc -l)
        echo -e "${GREEN}nginx is ${ratio}x faster${NC}"
    fi
}

# Test 1: File Size Performance
echo -e "${BLUE}Testing file size performance...${NC}"

print_results_table "Small File (index.html)" "index.html" "100" ""
print_results_table "Medium File (medium.txt)" "medium.txt" "100" ""
print_results_table "Large File (large.txt)" "large.txt" "100" ""

# Test 2: Concurrency Scaling
echo -e "\n${BLUE}Testing concurrency scaling with index.html...${NC}"

for concurrency in 1 10 50 100 200 500; do
    print_results_table "Concurrency $concurrency" "index.html" "$concurrency" ""
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
echo "- Look for failed requests (should be 0)"
echo "- Compare P99 latencies for tail performance"
echo "- Review logs in $LOG_DIR for detailed analysis"

echo -e "\n${YELLOW}Tip: Run 'ls -la $LOG_DIR' to see all generated log files${NC}"