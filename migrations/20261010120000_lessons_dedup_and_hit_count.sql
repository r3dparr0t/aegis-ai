-- dedup + hit_count برای lessons
-- نکته: این migration قبل از CREATE UNIQUE INDEX،
-- duplicate های (task_type, error_category) موجود را merge می‌کند.

-- ۱. ستون شمارنده
ALTER TABLE lessons ADD COLUMN hit_count INTEGER NOT NULL DEFAULT 1;

-- ۲. جدول موقت survivor ها (جدیدترین نسخه‌ی هر گروه)
CREATE TEMP TABLE lesson_survivors AS
SELECT
    task_type,
    error_category,
    (
        SELECT l2.id FROM lessons l2
        WHERE l2.task_type = l.task_type
          AND l2.error_category = l.error_category
        ORDER BY l2.created_at DESC, l2.rowid DESC
        LIMIT 1
    ) AS survivor_id,
    COUNT(*) AS total_count
FROM lessons l
GROUP BY task_type, error_category;

-- ۳. همه‌ی usage ها را به survivor منتقل کن
--    (قبل از DELETE — وگرنه FK از lesson_usage به lessons می‌شکند)
UPDATE lesson_usage
SET lesson_id = (
    SELECT s.survivor_id
    FROM lesson_survivors s
    JOIN lessons l ON l.id = lesson_usage.lesson_id
    WHERE s.task_type = l.task_type
      AND s.error_category = l.error_category
)
WHERE lesson_id IN (SELECT id FROM lessons);

-- ۴. hit_count survivor = کل duplicate های گروه
UPDATE lessons
SET hit_count = (
    SELECT s.total_count FROM lesson_survivors s
    WHERE s.survivor_id = lessons.id
)
WHERE id IN (SELECT survivor_id FROM lesson_survivors);

-- ۵. حذف duplicate های غیر survivor
DELETE FROM lessons
WHERE id NOT IN (SELECT survivor_id FROM lesson_survivors);

-- ۶. حالا unique index امن است
CREATE UNIQUE INDEX IF NOT EXISTS uq_lessons_task_error
    ON lessons(task_type, error_category);

-- ۷. پاک‌سازی
DROP TABLE lesson_survivors;
