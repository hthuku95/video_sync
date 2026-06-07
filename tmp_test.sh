#!/bin/bash
RESULT=$(curl -s -X POST http://localhost:3000/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"testadmin@videosync.test","password":"TestAdmin#2026"}')
TOKEN=$(echo "$RESULT" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")
echo "Token OK"
curl -s "http://localhost:3000/api/admin/prospects" \
  -H "Authorization: Bearer $TOKEN" | python3 -c "
import sys,json
data = json.load(sys.stdin)
for p in data['prospects']:
    sid = p.get('sample_delivery_id')
    if not sid:
        print(p['id'], p['display_name'], p.get('service_type','?'), p.get('external_url','?')[:60])
"
