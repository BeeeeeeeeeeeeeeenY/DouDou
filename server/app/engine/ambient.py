import time

import httpx

# 环境事实注入：天气（Open-Meteo，免 key）。失败静默跳过，绝不阻塞对话。
WEATHER_CITY = "深圳"
LAT, LON = 22.5429, 114.0596
TTL = 1800  # 正常缓存 30 分钟
RETRY_AFTER = 120  # 失败后 2 分钟内不重试

_cache: dict = {"ts": 0.0, "line": ""}

WMO = {
    0: "晴", 1: "基本晴", 2: "局部多云", 3: "阴", 45: "有雾", 48: "雾凇",
    51: "毛毛雨", 53: "毛毛雨", 55: "毛毛雨", 61: "小雨", 63: "中雨", 65: "大雨",
    66: "冻雨", 67: "冻雨", 71: "小雪", 73: "中雪", 75: "大雪", 77: "雪粒",
    80: "阵雨", 81: "阵雨", 82: "强阵雨", 85: "阵雪", 86: "阵雪",
    95: "雷阵雨", 96: "雷阵雨伴冰雹", 99: "雷阵雨伴冰雹",
}


async def weather_line() -> str:
    now = time.time()
    if now - _cache["ts"] < TTL:
        return _cache["line"]
    try:
        async with httpx.AsyncClient(timeout=4) as client:
            r = await client.get("https://api.open-meteo.com/v1/forecast", params={
                "latitude": LAT, "longitude": LON,
                "current": "temperature_2m,weather_code",
                "daily": "temperature_2m_max,temperature_2m_min,weather_code",
                "timezone": "Asia/Shanghai", "forecast_days": 1,
            })
        d = r.json()
        day = d["daily"]
        desc = WMO.get(day["weather_code"][0], "多云")
        line = (
            f"今天{WEATHER_CITY}天气：{desc}，气温 {round(day['temperature_2m_min'][0])}"
            f"~{round(day['temperature_2m_max'][0])} 度，现在 {round(d['current']['temperature_2m'])} 度。"
        )
        _cache.update(ts=now, line=line)
        return line
    except Exception:
        _cache.update(ts=now - TTL + RETRY_AFTER, line="")
        return ""
