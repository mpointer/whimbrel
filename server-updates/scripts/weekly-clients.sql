-- Weekly unique desktop clients, by platform, last 12 weeks.
-- Run (API token needs Account Analytics Read):
--   curl "https://api.cloudflare.com/client/v4/accounts/$CF_ACCOUNT_ID/analytics_engine/sql" \
--     -H "Authorization: Bearer $CF_API_TOKEN" \
--     --data-binary @server-updates/scripts/weekly-clients.sql
-- blob1 = weekly client id, blob2 = platform.
SELECT
  toStartOfWeek(timestamp, 1) AS week,
  blob2 AS platform,
  count(DISTINCT blob1) AS clients
FROM whimbrel_clients
WHERE timestamp > NOW() - INTERVAL '84' DAY
GROUP BY week, platform
ORDER BY week DESC, platform
