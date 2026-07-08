import sys

filepath = '/home/ubuntu/video_sync_integrations/YTDLPAPI/app/browserbase_strategy.py'

with open(filepath) as f:
    lines = f.readlines()

# Find the _download_with_ytdlp function definition
insert_after = None
for i, line in enumerate(lines):
    if 'async def _download_with_ytdlp' in line:
        insert_after = i - 1  # insert before the blank line before the function
        break

if insert_after is None:
    print("ERROR: could not find _download_with_ytdlp")
    sys.exit(1)

new_code = """\

    # Method 5: Parse Next.js RSC payload (__next_f script tags)
    # Kick uses Next.js App Router; stream URLs are often in __next_f payloads
    try:
        all_rsc = await page.evaluate('''
            () => {
                const scripts = document.querySelectorAll("script");
                let combined = "";
                for (const s of scripts) {
                    const t = s.textContent || "";
                    if (t.includes("__next_f")) {
                        combined += t;
                    }
                }
                return combined.substring(0, 500000);
            }
        ''')
        if all_rsc:
            urls = _extract_video_urls(all_rsc)
            if urls:
                m3u8 = [u for u in urls if ".m3u8" in u]
                if m3u8:
                    logger.info(f"Found HLS in __next_f RSC payload: {m3u8[0][:120]}")
                    return m3u8[0]
                if urls:
                    logger.info(f"Found video URL in __next_f RSC payload: {urls[0][:120]}")
                    return urls[0]
    except Exception as e:
        logger.debug(f"RSC payload check failed: {e}")

    # Method 6: Try window.__INITIAL_STATE__ and similar global stores
    try:
        storage_data = await page.evaluate('''
            () => {
                const data = {};
                try { data.window_data = JSON.stringify(
                    window.__INITIAL_STATE__ || window.__NEXT_DATA__ || null
                ); } catch(e) {}
                return JSON.stringify(data).substring(0, 50000);
            }
        ''')
        if storage_data:
            urls = _extract_video_urls(storage_data)
            if urls:
                m3u8 = [u for u in urls if ".m3u8" in u]
                if m3u8:
                    logger.info(f"Found HLS in window data: {m3u8[0][:120]}")
                    return m3u8[0]
    except Exception as e:
        logger.debug(f"Window data check failed: {e}")

"""

lines.insert(insert_after, new_code)

with open(filepath, 'w') as f:
    f.writelines(lines)

print('Fixed - inserted RSC payload parsing methods')
