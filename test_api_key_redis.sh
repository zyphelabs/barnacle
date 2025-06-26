#!/bin/bash

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

API_URL="http://localhost:3000/test"
VALID_KEY="valid-key-123"
PREMIUM_KEY="premium-key-456"
BASIC_KEY="basic-key-789"
INVALID_KEY="invalid-key-xyz"

# Rate limits (must match the Rust example)
RATE_LIMIT_VALID=3
RATE_LIMIT_PREMIUM=10
RATE_LIMIT_BASIC=1
WINDOW_SECONDS=6 # Must match the Rust example

echo -e "${CYAN}== Barnacle API Key + Redis Automated Tests ==${NC}"
echo -e "${CYAN}Testing multiple API keys with different rate limits${NC}"

# 1. Test: No API Key
echo -e "\n${YELLOW}Test 1: Request without API Key${NC}"
RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -X GET "$API_URL")
echo -e "$RESPONSE"
if echo "$RESPONSE" | grep -q "401"; then
    echo -e "${GREEN}✔ Expected: Rejected due to missing API Key${NC}"
else
    echo -e "${RED}✘ Failure: Should reject when API Key is missing${NC}"
fi

# 2. Test: Invalid API Key
echo -e "\n${YELLOW}Test 2: Request with invalid API Key${NC}"
RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -H "x-api-key: $INVALID_KEY" -X GET "$API_URL")
echo -e "$RESPONSE"
if echo "$RESPONSE" | grep -q "401"; then
    echo -e "${GREEN}✔ Expected: Rejected due to invalid API Key${NC}"
else
    echo -e "${RED}✘ Failure: Should reject invalid API Key${NC}"
fi

# 3. Test: Valid API Key (below rate limit)
echo -e "\n${YELLOW}Test 3: Request with valid API Key (below rate limit)${NC}"
RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -H "x-api-key: $VALID_KEY" -X GET "$API_URL")
echo -e "$RESPONSE"
if echo "$RESPONSE" | grep -q "200"; then
    echo -e "${GREEN}✔ Expected: Accepted with valid API Key${NC}"
else
    echo -e "${RED}✘ Failure: Should accept valid API Key${NC}"
fi

# 4. Test: Exceeding the rate limit for valid key
echo -e "\n${YELLOW}Test 4: Exceeding the rate limit with valid API Key (${RATE_LIMIT_VALID} requests)${NC}"
for i in $(seq 1 $((RATE_LIMIT_VALID+1))); do
    RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -H "x-api-key: $VALID_KEY" -X GET "$API_URL")
    echo -e "Attempt $i: $(echo "$RESPONSE" | grep 'Status:')"
done

LAST_STATUS=$(echo "$RESPONSE" | grep 'Status:' | awk '{print $2}')
if [ "$LAST_STATUS" == "429" ]; then
    echo -e "${GREEN}✔ Expected: Rate limit reached, request rejected${NC}"
else
    echo -e "${RED}✘ Failure: Should reject after reaching the rate limit${NC}"
fi

# 5. Test: Premium API Key with higher rate limit
echo -e "\n${YELLOW}Test 5: Premium API Key with higher rate limit (${RATE_LIMIT_PREMIUM} requests)${NC}"
for i in $(seq 1 $RATE_LIMIT_PREMIUM); do
    RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -H "x-api-key: $PREMIUM_KEY" -X GET "$API_URL")
    STATUS=$(echo "$RESPONSE" | grep 'Status:' | awk '{print $2}')
    echo -e "Attempt $i: Status: $STATUS"
    if [ "$STATUS" != "200" ]; then
        echo -e "${RED}✘ Failure: Premium key should allow ${RATE_LIMIT_PREMIUM} requests${NC}"
        break
    fi
done

if [ "$STATUS" == "200" ]; then
    echo -e "${GREEN}✔ Expected: Premium key allows ${RATE_LIMIT_PREMIUM} requests${NC}"
fi

# 6. Test: Basic API Key with very low rate limit
echo -e "\n${YELLOW}Test 6: Basic API Key with very low rate limit (${RATE_LIMIT_BASIC} request)${NC}"
RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -H "x-api-key: $BASIC_KEY" -X GET "$API_URL")
echo -e "First request: $(echo "$RESPONSE" | grep 'Status:')"
if echo "$RESPONSE" | grep -q "200"; then
    echo -e "${GREEN}✔ Expected: Basic key allows first request${NC}"
    
    # Second request should be rejected
    RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -H "x-api-key: $BASIC_KEY" -X GET "$API_URL")
    echo -e "Second request: $(echo "$RESPONSE" | grep 'Status:')"
    if echo "$RESPONSE" | grep -q "429"; then
        echo -e "${GREEN}✔ Expected: Basic key rejects second request${NC}"
    else
        echo -e "${RED}✘ Failure: Basic key should reject second request${NC}"
    fi
else
    echo -e "${RED}✘ Failure: Basic key should allow first request${NC}"
fi

# 7. Test: Rate limit reset after window expiration
echo -e "\n${YELLOW}Test 7: Rate limit reset after window expiration${NC}"
echo -e "${CYAN}Waiting for rate limit window to reset (${WINDOW_SECONDS} seconds)...${NC}"

# Wait for the window to expire (with a small buffer to ensure reset)
sleep $((WINDOW_SECONDS + 2))

echo -e "${CYAN}Window expired, testing if API keys can be used again...${NC}"

# Test all keys again
for key_name in "$VALID_KEY" "$PREMIUM_KEY" "$BASIC_KEY"; do
    echo -e "${CYAN}Testing $key_name after reset...${NC}"
    RESPONSE=$(curl -s -w "\nStatus: %{http_code}\n" -H "x-api-key: $key_name" -X GET "$API_URL")
    
    if echo "$RESPONSE" | grep -q "200"; then
        echo -e "${GREEN}✔ Expected: $key_name can be used again after window reset${NC}"
    else
        echo -e "${RED}✘ Failure: $key_name should be usable again after window reset${NC}"
    fi
done

echo -e "\n${CYAN}== End of tests ==${NC}" 