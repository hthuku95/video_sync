#!/bin/bash
set -e
RESULT=$(curl -s -X POST http://localhost:3000/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"testadmin@videosync.test","password":"TestAdmin#2026"}')
TOKEN=$(echo "$RESULT" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
echo "Token OK"
curl -s -X POST "http://localhost:3000/api/admin/prospects/59bb9021-2194-4e1d-aa17-3e083dc020ee/generate-sample-pack" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{}'
echo ""
echo "Done"
