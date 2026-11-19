-- خروجی خام مدل (قبل از overwrite با بدنه‌ی پاسخ HTTP) را جداگانه نگه می‌دارد.
-- ستون output همچنان بدنه‌ی پاسخ سرور هدف را نگه می‌دارد.
ALTER TABLE attempts ADD COLUMN model_output TEXT NOT NULL DEFAULT '';
