#!/usr/bin/env python3
"""Generate test .po catalogs (15 languages + en + pseudo-locale) and compile them to .mo.

Test strings only — written for rendering/shaping checks, NOT reviewed by native speakers.
Output: i18n/po/<lang>.po and locale/<lang>/LC_MESSAGES/junc-proto.mo
usage: python3 i18n/make_catalogs.py
"""
import os, re, subprocess, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
KEYS = [  # (context, msgid)
    ("Card", "Enable {0}"), ("Card", "Volume of {0}"), ("Card", "Level of {0}"),
    ("MainWindow", "Add input"), ("MainWindow", "Search streams and devices…"), ("MainWindow", "Close"),
    ("MainWindow", "Junctions"), ("MainWindow", "New junction"), ("MainWindow", "Add input…"),
    ("MainWindow", "Inputs"), ("MainWindow", "Listen"), ("MainWindow", "Bus"),
    ("MainWindow", "Translated sample: {0}"), ("MainWindow", "sample-sentence"),
]
PLURAL = ("MainWindow", "{n} input connected", "{n} inputs connected")

# gettext Plural-Forms per language (CLDR categories noted where gettext differs)
PF = {
    "ar": "nplurals=6; plural=(n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5);",
    "he": "nplurals=2; plural=(n != 1);", "fa": "nplurals=2; plural=(n > 1);", "ur": "nplurals=2; plural=(n != 1);",
    "hi": "nplurals=2; plural=(n != 1);", "bn": "nplurals=2; plural=(n != 1);", "ta": "nplurals=2; plural=(n != 1);",
    "si": "nplurals=2; plural=(n != 1);", "th": "nplurals=1; plural=0;", "ja": "nplurals=1; plural=0;",
    "zh_CN": "nplurals=1; plural=0;", "ko": "nplurals=1; plural=0;",
    # CLDR ru has one/few/many/other; "other" only applies to fractions, which gettext's integer n cannot express.
    "ru": "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
    "de": "nplurals=2; plural=(n != 1);", "fi": "nplurals=2; plural=(n != 1);", "en": "nplurals=2; plural=(n != 1);",
    "en_XA": "nplurals=2; plural=(n != 1);",
}

T = {  # order follows KEYS, then plural forms list
 "en": ["Enable {0}", "Volume of {0}", "Level of {0}", "Add input", "Search streams and devices…", "Close", "Junctions",
        "New junction", "Add input…", "Inputs", "Listen", "Bus", "Translated sample: {0}",
        "Press the switch to route audio through the junction to headphones and streaming software.",
        ["{n} input connected", "{n} inputs connected"]],
 "ar": ["تفعيل {0}", "مستوى صوت {0}", "مستوى إشارة {0}", "إضافة مدخل", "ابحث في التدفقات والأجهزة…", "إغلاق", "الوصلات",
        "وصلة جديدة", "إضافة مدخل…", "المدخلات", "الاستماع", "الناقل", "نص تجريبي: {0}",
        "اضغط على المفتاح لتوجيه الصوت عبر الوصلة إلى سماعات الرأس وبرنامج البث.",
        ["لا توجد مدخلات متصلة", "مدخل واحد متصل", "مدخلان متصلان", "{n} مدخلات متصلة", "{n} مدخلًا متصلًا", "{n} مدخل متصل"]],
 "he": ["הפעלת {0}", "עוצמת {0}", "רמת {0}", "הוספת קלט", "חיפוש זרמים והתקנים…", "סגירה", "צמתים",
        "צומת חדש", "הוספת קלט…", "קלטים", "האזנה", "אפיק", "דוגמה מתורגמת: {0}",
        "לחצו על המתג כדי לנתב את השמע דרך הצומת לאוזניות ולתוכנת השידור.",
        ["קלט אחד מחובר", "{n} קלטים מחוברים"]],
 "fa": ["فعال‌سازی {0}", "بلندی صدای {0}", "سطح {0}", "افزودن ورودی", "جستجوی جریان‌ها و دستگاه‌ها…", "بستن", "اتصال‌ها",
        "اتصال جدید", "افزودن ورودی…", "ورودی‌ها", "شنیدن", "گذرگاه", "نمونهٔ ترجمه‌شده: {0}",
        "برای هدایت صدا از طریق اتصال، کلید را بزنید. نسخهٔ ۱۲۳ (ارقام فارسی که مترجم نوشته است).",
        ["{n} ورودی متصل", "{n} ورودی متصل"]],
 "ur": ["{0} فعال کریں", "{0} کی آواز", "{0} کی سطح", "ان پٹ شامل کریں", "سٹریمز اور آلات تلاش کریں…", "بند کریں", "جنکشنز",
        "نیا جنکشن", "ان پٹ شامل کریں…", "ان پٹس", "سنیں", "بس", "ترجمہ شدہ نمونہ: {0}",
        "آواز کو جنکشن کے ذریعے ہیڈفون تک بھیجنے کے لیے سوئچ دبائیں۔",
        ["{n} ان پٹ منسلک ہے", "{n} ان پٹ منسلک ہیں"]],
 "hi": ["{0} चालू करें", "{0} का वॉल्यूम", "{0} का स्तर", "इनपुट जोड़ें", "स्ट्रीम और डिवाइस खोजें…", "बंद करें", "जंक्शन",
        "नया जंक्शन", "इनपुट जोड़ें…", "इनपुट", "सुनें", "बस", "अनूदित नमूना: {0}",
        "ध्वनि को जंक्शन से भेजने के लिए स्विच दबाएँ — क्षेत्र, द्वितीय, स्त्री, श्रृंखला।",
        ["{n} इनपुट जुड़ा है", "{n} इनपुट जुड़े हैं"]],
 "bn": ["{0} চালু করুন", "{0}-এর ভলিউম", "{0}-এর লেভেল", "ইনপুট যোগ করুন", "স্ট্রিম ও ডিভাইস খুঁজুন…", "বন্ধ করুন", "জংশন",
        "নতুন জংশন", "ইনপুট যোগ করুন…", "ইনপুট", "শুনুন", "বাস", "অনূদিত নমুনা: {0}",
        "জংশনের মাধ্যমে শব্দ পাঠাতে সুইচ চাপুন — ক্ষমা, স্ত্রী, যুক্তাক্ষর।",
        ["{n}টি ইনপুট সংযুক্ত", "{n}টি ইনপুট সংযুক্ত"]],
 "ta": ["{0} ஐ இயக்கு", "{0} ஒலியளவு", "{0} நிலை", "உள்ளீட்டைச் சேர்", "ஸ்ட்ரீம்கள், சாதனங்களைத் தேடு…", "மூடு", "சந்திப்புகள்",
        "புதிய சந்திப்பு", "உள்ளீட்டைச் சேர்…", "உள்ளீடுகள்", "கேள்", "பஸ்", "மொழிபெயர்க்கப்பட்ட மாதிரி: {0}",
        "சந்திப்பு வழியாக ஒலியை அனுப்ப சுவிட்சை அழுத்தவும்; க்ஷ, ஸ்ரீ.",
        ["{n} உள்ளீடு இணைக்கப்பட்டுள்ளது", "{n} உள்ளீடுகள் இணைக்கப்பட்டுள்ளன"]],
 "si": ["{0} සක්‍රිය කරන්න", "{0} හි හඬ", "{0} මට්ටම", "ආදානය එක් කරන්න", "ප්‍රවාහ සහ උපාංග සොයන්න…", "වසන්න", "සන්ධි",
        "නව සන්ධිය", "ආදානය එක් කරන්න…", "ආදාන", "සවන් දෙන්න", "බස්", "පරිවර්තිත නියැදිය: {0}",
        "සන්ධිය හරහා ශබ්දය යැවීමට ස්විචය ඔබන්න — ශ්‍රී ලංකා, ක්‍ෂ.",
        ["ආදාන {n} ක් සම්බන්ධයි", "ආදාන {n} ක් සම්බන්ධයි"]],
 "th": ["เปิดใช้ {0}", "ระดับเสียงของ {0}", "ระดับสัญญาณของ {0}", "เพิ่มอินพุต", "ค้นหาสตรีมและอุปกรณ์…", "ปิด", "จุดเชื่อมต่อ",
        "จุดเชื่อมต่อใหม่", "เพิ่มอินพุต…", "อินพุต", "ฟัง", "บัส", "ตัวอย่างคำแปล: {0}",
        "กดสวิตช์เพื่อส่งเสียงผ่านจุดเชื่อมต่อนี้ไปยังหูฟังและโปรแกรมถ่ายทอดสดทั้งหมดที่เลือกไว้ในหน้าต่างนี้โดยอัตโนมัติ",
        ["เชื่อมต่ออินพุต {n} รายการ"]],
 "ja": ["{0} を有効化", "{0} の音量", "{0} のレベル", "入力を追加", "ストリームとデバイスを検索…", "閉じる", "ジャンクション",
        "新しいジャンクション", "入力を追加…", "入力", "モニター", "バス", "翻訳サンプル: {0}",
        "スイッチを押すと、ジャンクション経由でヘッドホンと配信ソフトに音声がルーティングされます。",
        ["{n} 個の入力が接続済み"]],
 "zh_CN": ["启用 {0}", "{0} 的音量", "{0} 的电平", "添加输入", "搜索音频流和设备…", "关闭", "节点",
        "新建节点", "添加输入…", "输入", "监听", "总线", "翻译示例：{0}",
        "按下开关，即可通过节点将音频路由到耳机和直播软件。",
        ["已连接 {n} 个输入"]],
 "ko": ["{0} 사용", "{0} 볼륨", "{0} 레벨", "입력 추가", "스트림 및 장치 검색…", "닫기", "정션",
        "새 정션", "입력 추가…", "입력", "모니터링", "버스", "번역 예시: {0}",
        "스위치를 누르면 정션을 통해 헤드폰과 방송 프로그램으로 오디오가 전달됩니다.",
        ["입력 {n}개 연결됨"]],
 "ru": ["Включить {0}", "Громкость: {0}", "Уровень: {0}", "Добавить вход", "Поиск потоков и устройств…", "Закрыть", "Узлы",
        "Новый узел", "Добавить вход…", "Входы", "Прослушивание", "Шина", "Пример перевода: {0}",
        "Нажмите переключатель, чтобы направить звук через узел в наушники и программу трансляции.",
        ["{n} вход подключён", "{n} входа подключены", "{n} входов подключено"]],
 "de": ["{0} aktivieren", "Lautstärke von {0}", "Pegel von {0}", "Eingang hinzufügen", "Streams und Geräte durchsuchen…", "Schließen",
        "Verbindungen", "Neue Verbindung", "Eingang hinzufügen…", "Eingänge", "Abhören", "Bus", "Übersetztes Beispiel: {0}",
        "Drücken Sie den Schalter, um den Ton über die Verbindung an Kopfhörer und Streaming-Software zu leiten. Lautstärkeregelungsvoreinstellungen.",
        ["{n} Eingang verbunden", "{n} Eingänge verbunden"]],
 "fi": ["Ota {0} käyttöön", "Kohteen {0} äänenvoimakkuus", "Kohteen {0} taso", "Lisää tulo", "Hae virtoja ja laitteita…", "Sulje",
        "Liitokset", "Uusi liitos", "Lisää tulo…", "Tulot", "Kuuntelu", "Väylä", "Käännetty esimerkki: {0}",
        "Paina kytkintä ohjataksesi äänen liitoksen kautta kuulokkeisiin ja lähetysohjelmaan. Äänenvoimakkuudensäätöasetukset.",
        ["{n} tulo yhdistetty", "{n} tuloa yhdistetty"]],
}

ACC = str.maketrans("aceinousyACEINOUSY", "åçéîñöûšýÅÇÉÎÑÖÛŠÝ")


def pseudo(s: str) -> str:
    """Accent letters outside {placeholders}, pad to +40% length, bracket so truncation is visible."""
    parts = re.split(r"(\{[^}]*\})", s)
    body = "".join(p if p.startswith("{") else p.translate(ACC) for p in parts)
    pad = max(1, round(len(s) * 0.4))
    return "⟦" + body + " " + "·" * (pad - 1) + "⟧"


def esc(s): return s.replace("\\", "\\\\").replace('"', '\\"')


def write(lang, vals):
    out = [f'msgid ""\nmsgstr ""\n"Project-Id-Version: junc-proto\\n"\n"PO-Revision-Date: 2026-11-24\\n"\n"Last-Translator: test strings (unreviewed)\\n"\n"Language-Team: none\\n"\n"MIME-Version: 1.0\\n"\n"Content-Transfer-Encoding: 8bit\\n"\n"Content-Type: text/plain; charset=UTF-8\\n"\n"Language: {lang}\\n"\n"Plural-Forms: {PF[lang]}\\n"\n']
    for (ctx, mid), v in zip(KEYS, vals):
        out.append(f'msgctxt "{ctx}"\nmsgid "{esc(mid)}"\nmsgstr "{esc(v)}"\n')
    forms = vals[len(KEYS)]
    pl = f'msgctxt "{PLURAL[0]}"\nmsgid "{esc(PLURAL[1])}"\nmsgid_plural "{esc(PLURAL[2])}"\n'
    pl += "".join(f'msgstr[{i}] "{esc(f)}"\n' for i, f in enumerate(forms))
    out.append(pl)
    po = ROOT / "i18n" / "po" / f"{lang}.po"; po.parent.mkdir(parents=True, exist_ok=True)
    po.write_text("\n".join(out), encoding="utf-8")
    mo = ROOT / "locale" / lang / "LC_MESSAGES" / "junc-proto.mo"; mo.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["msgfmt", "--check", "-o", str(mo), str(po)], check=True)


for lang, vals in T.items():
    write(lang, vals)
en = T["en"]
write("en_XA", [pseudo(v) for v in en[:len(KEYS)]] + [[pseudo(f) for f in en[len(KEYS)]]])
print("catalogs:", " ".join(sorted(list(T) + ["en_XA"])))
