-- ثبت اینکه یه اجرا با کمکِ hint دستی یا fallback مدل پاس شده، نه خودش تنها
ALTER TABLE executions ADD COLUMN hint_used BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE executions ADD COLUMN fallback_used BOOLEAN NOT NULL DEFAULT 0;
