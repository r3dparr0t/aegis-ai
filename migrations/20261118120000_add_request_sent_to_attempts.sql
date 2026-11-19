-- درخواست نهایی‌ای که به سرور هدف رفته (payload پس از transform) را نگه می‌دارد.
-- در مسیرهایی که به execute نرسیده (shell_failed، json_parse_failed) مقدار خالی است.
ALTER TABLE attempts ADD COLUMN request_sent TEXT NOT NULL DEFAULT '';
