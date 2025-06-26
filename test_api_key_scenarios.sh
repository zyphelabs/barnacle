#!/bin/bash

# API Key Validation & Rate Limiting Test Script
# This script tests both API key validation and rate limiting scenarios

echo "🔐 API Key Validation & Rate Limiting Test Suite"
echo "================================================"
echo

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

BASE_URL="http://localhost:3000"

# Function to make HTTP requests and show results
make_request() {
    local method=$1
    local endpoint=$2
    local api_key=$3
    local description=$4
    
    echo -e "${YELLOW}${description}${NC}"
    
    if [ -n "$api_key" ]; then
        response=$(curl -s -w "\n%{http_code}" -X "$method" \
            -H "x-api-key: $api_key" \
            "$BASE_URL$endpoint")
    else
        response=$(curl -s -w "\n%{http_code}" -X "$method" \
            "$BASE_URL$endpoint")
    fi
    
    # Split response and status code
    body=$(echo "$response" | sed '$d')
    status=$(echo "$response" | tail -n 1)
    
    # Color code the status
    if [ "$status" -eq 200 ]; then
        echo -e "Status: ${GREEN}$status${NC}"
    elif [ "$status" -eq 401 ]; then
        echo -e "Status: ${RED}$status${NC}"
    elif [ "$status" -eq 429 ]; then
        echo -e "Status: ${PURPLE}$status${NC}"
    else
        echo -e "Status: ${YELLOW}$status${NC}"
    fi
    
    echo "Response: $body"
    echo "---"
    echo
}

# Function to test rate limiting for a specific key
test_rate_limiting() {
    local api_key=$1
    local max_requests=$2
    local description=$3
    
    echo -e "${BLUE}${description}${NC}"
    echo "Testing rate limiting: $max_requests requests allowed"
    echo
    
    for i in $(seq 1 $((max_requests + 1))); do
        if [ $i -le $max_requests ]; then
            expected_status="200"
            expected_result="should succeed"
        else
            expected_status="429"
            expected_result="should be rate limited"
        fi
        
        echo -e "${CYAN}Request $i (${expected_result}):${NC}"
        make_request "GET" "/api/data" "$api_key" "Rate limit test $i/$((max_requests + 1))"
        
        # Small delay between requests
        sleep 0.5
    done
    
    echo -e "${GREEN}Rate limiting test completed for $api_key${NC}"
    echo "================================================"
    echo
}

# Test 1: Missing API Key
echo -e "${BLUE}Test 1: Missing API Key${NC}"
echo "Testing requests without API key header..."
echo

make_request "GET" "/api/data" "" "Request without API key"
make_request "GET" "/api/status" "" "Status request without API key"

echo -e "${GREEN}Expected: Both requests should return 401 Unauthorized${NC}"
echo

# Test 2: Invalid API Key
echo -e "${BLUE}Test 2: Invalid API Key${NC}"
echo "Testing requests with invalid API keys..."
echo

make_request "GET" "/api/data" "invalid_key" "Request with invalid API key"
make_request "GET" "/api/data" "nonexistent_key" "Request with nonexistent API key"
make_request "GET" "/api/data" "wrong_key_123" "Request with wrong API key"

echo -e "${GREEN}Expected: All requests should return 401 Unauthorized${NC}"
echo

# Test 3: Valid API Key - Basic Tier
echo -e "${BLUE}Test 3: Valid API Key - Basic Tier (3 requests/60s)${NC}"
echo "Testing basic_key with rate limiting..."
echo

test_rate_limiting "basic_key" 3 "Basic API Key Rate Limiting"

# Test 4: Valid API Key - Strict Tier
echo -e "${BLUE}Test 4: Valid API Key - Strict Tier (2 requests/30s)${NC}"
echo "Testing strict_key with rate limiting..."
echo

test_rate_limiting "strict_key" 2 "Strict API Key Rate Limiting"

# Test 5: Valid API Key - Premium Tier
echo -e "${BLUE}Test 5: Valid API Key - Premium Tier (10 requests/60s)${NC}"
echo "Testing premium_key with rate limiting..."
echo

test_rate_limiting "premium_key" 10 "Premium API Key Rate Limiting"

# Test 6: Health endpoint (no API key required)
echo -e "${BLUE}Test 6: Health Endpoint (No API Key Required)${NC}"
echo "Testing health endpoint without API key..."
echo

make_request "GET" "/health" "" "Health check without API key"

echo -e "${GREEN}Expected: Should return 200 OK (health endpoint doesn't require API key)${NC}"
echo

# Test 7: Mixed scenarios
echo -e "${BLUE}Test 7: Mixed Scenarios${NC}"
echo "Testing various combinations..."
echo

echo -e "${CYAN}Testing valid key after invalid key:${NC}"
make_request "GET" "/api/data" "invalid_key" "Invalid key first"
make_request "GET" "/api/data" "basic_key" "Valid key second"

echo -e "${CYAN}Testing different endpoints with same key:${NC}"
make_request "GET" "/api/data" "basic_key" "Data endpoint"
make_request "GET" "/api/status" "basic_key" "Status endpoint"

echo -e "${CYAN}Testing rate limit headers:${NC}"
response=$(curl -s -i -H "x-api-key: strict_key" "$BASE_URL/api/data")
echo "Response headers:"
echo "$response" | head -20

echo
echo -e "${BLUE}Test Suite Complete!${NC}"
echo
echo "📊 Summary of Expected Results:"
echo "  ✅ Missing API key → 401 Unauthorized"
echo "  ✅ Invalid API key → 401 Unauthorized"
echo "  ✅ Rate limit exceeded → 429 Too Many Requests"
echo "  ✅ Valid API key within limits → 200 OK"
echo "  ✅ Health endpoint → 200 OK (no API key required)"
echo
echo "🚀 To run the server:"
echo "  cargo run --example api_key_validation"
echo
echo "🔧 To test with different configurations, modify the example file." 