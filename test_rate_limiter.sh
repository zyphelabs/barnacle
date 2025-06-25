#!/bin/bash

# Barnacle Rate Limiter Test Script
# This script demonstrates various rate limiting scenarios

echo "🧪 Barnacle Rate Limiter Test Suite"
echo "===================================="
echo

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

BASE_URL="http://localhost:3000"

# Function to make HTTP requests and show results
make_request() {
    local method=$1
    local endpoint=$2
    local data=$3
    local extra_header=$4
    
    if [ -n "$data" ]; then
        if [ -n "$extra_header" ]; then
            response=$(curl -s -w "\n%{http_code}" -X "$method" \
                -H "Content-Type: application/json" \
                -H "$extra_header" \
                -d "$data" \
                "$BASE_URL$endpoint")
        else
            response=$(curl -s -w "\n%{http_code}" -X "$method" \
                -H "Content-Type: application/json" \
                -d "$data" \
                "$BASE_URL$endpoint")
        fi
    else
        response=$(curl -s -w "\n%{http_code}" -X "$method" \
            "$BASE_URL$endpoint")
    fi
    
    # Split response and status code
    body=$(echo "$response" | sed '$d')
    status=$(echo "$response" | tail -n 1)
    
    echo "Status: $status"
    echo "Response: $body"
    echo "---"
}

# Test 1: Basic rate limiting
echo -e "${BLUE}Test 1: Basic Rate Limiting (5 requests per minute)${NC}"
echo "Making 6 requests to /api/strict endpoint..."
echo

for i in {1..6}; do
    echo -e "${YELLOW}Request $i:${NC}"
    make_request "GET" "/api/strict"
    sleep 0.5
done

echo -e "${GREEN}Expected: First 5 requests should succeed (200), 6th should fail (429)${NC}"
echo

# Test 2: Different rate limits
echo -e "${BLUE}Test 2: Different Rate Limits${NC}"
echo "Testing moderate endpoint (20 requests per minute)..."
echo

for i in {1..3}; do
    echo -e "${YELLOW}Request $i:${NC}"
    make_request "GET" "/api/moderate"
    sleep 0.5
done

echo -e "${GREEN}Expected: All requests should succeed (200)${NC}"
echo

# Test 3: Login rate limiting with wrong password until 429 error
echo -e "${BLUE}Test 3: Login Rate Limiting (3 attempts per 5 minutes)${NC}"
echo "Testing login endpoint with wrong password..."
echo

for i in {1..4}; do
    echo -e "${YELLOW}Login attempt $i:${NC}"
    make_request "POST" "/api/login" '{"email":"test_fail@example.com","password":"wrong_password"}' "X-Login-Email: test_fail@example.com"
    sleep 0.5
done

echo -e "${GREEN}Expected: First 3 attempts should fail (401), 4th should be rate limited (429)${NC}"
echo

echo "Resetting rate limit for test_fail@example.com..."
echo
make_request "POST" "/api/reset/email/test_fail@example.com"
echo -e "${GREEN}Expected: Should succeed (200)${NC}"
echo


echo -e "${YELLOW}Login attempt afer reset $i:${NC}"
make_request "POST" "/api/login" '{"email":"test_fail@example.com","password":"wrong_password"}' "X-Login-Email: test_fail@example.com"
sleep 0.5
echo -e "${GREEN}Expected: Should succeed request but return unauthorized (401)${NC}"
echo

# Test 3.1: Login rate limiting with wrong password
echo -e "${BLUE}Test 3.1: Login Rate Limiting (3 attempts per 5 minutes)${NC}"
echo "Testing login endpoint with wrong password..."
echo

for i in {1..2}; do
    echo -e "${YELLOW}Login attempt $i:${NC}"
    make_request "POST" "/api/login" '{"email":"test@example.com","password":"wrong_password"}' "X-Login-Email: test@example.com"
    sleep 0.5
done

echo -e "${GREEN}Expected: First 2 attempts should fail (401)${NC}"
echo

# Test 4: Successful login with rate limit reset
echo -e "${BLUE}Test 4: Successful Login with Rate Limit Reset${NC}"
echo "Testing login with correct password..."
echo

make_request "POST" "/api/login" '{"email":"test@example.com","password":"correct_password"}' "X-Login-Email: test@example.com"
make_request "POST" "/api/login" '{"email":"test@example.com","password":"correct_password"}' "X-Login-Email: test@example.com"
make_request "POST" "/api/login" '{"email":"test@example.com","password":"correct_password"}' "X-Login-Email: test@example.com"


echo -e "${GREEN}Expected: Should succeed (200) and reset rate limit${NC}"
echo

# Test 5: Manual rate limit reset
echo -e "${BLUE}Test 5: Manual Rate Limit Reset${NC}"
echo "Resetting rate limit for test@example.com..."
echo

make_request "POST" "/api/reset/email/test@example.com"

echo -e "${GREEN}Expected: Should succeed (200)${NC}"
echo

# Test 6: Status endpoint
echo -e "${BLUE}Test 6: Status Endpoint${NC}"
echo "Checking rate limiter status..."
echo

make_request "GET" "/api/status"

echo -e "${GREEN}Expected: Should succeed (200)${NC}"
echo

echo -e "${BLUE}Test Suite Complete!${NC}"
echo
echo "To run the server, use:"
echo "cargo run --example advanced"
echo
echo "To test with Redis, use:"
echo "cargo run --example basic" 