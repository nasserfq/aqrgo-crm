-- Building details that don't fit the original Word table columns.
ALTER TABLE listings
    ADD COLUMN rooms     TEXT NOT NULL DEFAULT '',  -- عدد الغرف
    ADD COLUMN bathrooms TEXT NOT NULL DEFAULT '',  -- دورات المياه
    ADD COLUMN floors    TEXT NOT NULL DEFAULT '',  -- الأدوار
    ADD COLUMN units     TEXT NOT NULL DEFAULT '',  -- الوحدات
    ADD COLUMN income    TEXT NOT NULL DEFAULT '',  -- الدخل والعقود
    ADD COLUMN details   TEXT NOT NULL DEFAULT '';  -- تفاصيل إضافية
