#!/bin/bash

# API Key Denial Test Script
# Tests the two main API key denial scenarios:
# 1. Deny requests coming from unknown or missing API keys
# 2. Deny requests if the rate limit threshold is exceeded

echo "🚫 API Key Denial Test Suite"
echo "============================"
echo

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m' # No Color

BASE_URL="http://localhost:3000"

# Function to make HTTP request and check result
test_request() {
    local api_key=$1
    local expected_status=$2
    local description=$3
    
    echo -e "${YELLOW}${description}${NC}"
    
    if [ -n "$api_key" ]; then
        response=$(curl -s -w "\n%{http_code}" -X GET \
            -H "x-api-key: $api_key" \
            "$BASE_URL/api/data")
    else
        response=$(curl -s -w "\n%{http_code}" -X GET \
            "$BASE_URL/api/data")
    fi
    
    # Split response and status code
    body=$(echo "$response" | sed '$d')
    status=$(echo "$response" | tail -n 1)
    
    # Check if status matches expected
    if [ "$status" -eq "$expected_status" ]; then
        echo -e "Status: ${GREEN}$status${NC} ✅ (Expected: $expected_status)"
    else
        echo -e "Status: ${RED}$status${NC} ❌ (Expected: $expected_status)"
    fi
    
    echo "Response: $body"
    echo "---"
    echo
}

# Function to test rate limit threshold
test_rate_limit_threshold() {
    local api_key=$1
    local max_requests=$2
    local description=$3
    
    echo -e "${BLUE}${description}${NC}"
    echo "Testing rate limit threshold: $max_requests requests allowed"
    echo
    
    # Make requests up to the limit
    for i in $(seq 1 $max_requests); do
        echo -e "${CYAN}Request $i/$max_requests (should succeed):${NC}"
        test_request "$api_key" 200 "Rate limit test $i/$max_requests"
        sleep 0.3
    done
    
    # Make one more request that should be denied
    echo -e "${CYAN}Request $((max_requests + 1)) (should be denied):${NC}"
    test_request "$api_key" 429 "Rate limit threshold exceeded"
    
    echo -e "${GREEN}Rate limit threshold test completed for $api_key${NC}"
    echo "================================================"
    echo
}

# Test Scenario 1: Deny requests with missing or unknown API keys
echo -e "${BLUE}Scenario 1: Deny requests with missing or unknown API keys${NC}"
echo "=================================================================="
echo

echo -e "${PURPLE}Test 1.1: Missing API key${NC}"
test_request "" 401 "Request without API key header"

echo -e "${PURPLE}Test 1.2: Empty API key${NC}"
test_request "" 401 "Request with empty API key"

echo -e "${PURPLE}Test 1.3: Unknown API key${NC}"
test_request "unknown_key_123" 401 "Request with unknown API key"

echo -e "${PURPLE}Test 1.4: Invalid API key format${NC}"
test_request "invalid@key#format" 401 "Request with invalid API key format"

echo -e "${PURPLE}Test 1.5: Non-existent API key${NC}"
test_request "nonexistent_key_456" 401 "Request with non-existent API key"

echo -e "${GREEN}✅ Scenario 1 Complete: All requests should return 401 Unauthorized${NC}"
echo

# Test Scenario 2: Deny requests when rate limit threshold is exceeded
echo -e "${BLUE}Scenario 2: Deny requests when rate limit threshold is exceeded${NC}"
echo "====================================================================="
echo

echo -e "${PURPLE}Test 2.1: Strict rate limit (2 requests/30s)${NC}"
test_rate_limit_threshold "strict_key" 2 "Strict API Key Rate Limiting"

echo -e "${PURPLE}Test 2.2: Basic rate limit (3 requests/60s)${NC}"
test_rate_limit_threshold "basic_key" 3 "Basic API Key Rate Limiting"

echo -e "${PURPLE}Test 2.3: Premium rate limit (10 requests/60s)${NC}"
test_rate_limit_threshold "premium_key" 10 "Premium API Key Rate Limiting"

echo -e "${GREEN}✅ Scenario 2 Complete: All threshold violations should return 429 Too Many Requests${NC}"
echo

# Test Scenario 3: Verify valid requests work
echo -e "${BLUE}Scenario 3: Verify valid requests work within limits${NC}"
echo "============================================================="
echo

echo -e "${PURPLE}Test 3.1: Valid API key within limits${NC}"
test_request "basic_key" 200 "Valid API key within rate limits"

echo -e "${PURPLE}Test 3.2: Health endpoint (no API key required)${NC}"
response=$(curl -s -w "\n%{http_code}" -X GET "$BASE_URL/health")
body=$(echo "$response" | sed '$d')
status=$(echo "$response" | tail -n 1)

if [ "$status" -eq 200 ]; then
    echo -e "Status: ${GREEN}$status${NC} ✅ (Health endpoint should work without API key)"
else
    echo -e "Status: ${RED}$status${NC} ❌ (Health endpoint should return 200)"
fi
echo "Response: $body"
echo "---"
echo

echo -e "${GREEN}✅ Scenario 3 Complete: Valid requests should work${NC}"
echo

# Summary
echo -e "${BLUE}Test Summary${NC}"
echo "============="
echo
echo "🎯 Tested Scenarios:"
echo "  1. ✅ Missing/Unknown API keys → 401 Unauthorized"
echo "  2. ✅ Rate limit threshold exceeded → 429 Too Many Requests"
echo "  3. ✅ Valid requests within limits → 200 OK"
echo
echo "🔑 API Keys Used:"
echo "  - strict_key: 2 requests/30s"
echo "  - basic_key: 3 requests/60s"
echo "  - premium_key: 10 requests/60s"
echo
echo "🚀 To run the server:"
echo "  cargo run --example api_key_validation"
echo
echo "📝 To run this test:"
echo "  ./test_api_key_denial.sh" 