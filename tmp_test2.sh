#!/bin/bash
set -e
RESULT=$(curl -s -X POST http://localhost:3000/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"testadmin@videosync.test","password":"TestAdmin#2026"}')
TOKEN=$(echo "$RESULT" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
echo "Token: ${TOKEN:0:20}..."
curl -s -X POST "http://localhost:3000/api/admin/prospects/7eda573a-2ba1-42cf-bcd9-c93912a5fcdc/generate-sample-pack" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}'
echo ""
echo "Done"
