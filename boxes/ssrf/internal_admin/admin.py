from flask import Flask, jsonify
import time

app = Flask(__name__)

@app.route('/admin/secret-flag', methods=['GET'])
def secret_flag():
    return jsonify({
        "status": "UNLOCKED",
        "flag": "FLAG{Aegis_AI_SSRF_Discovery_Success}",
        "internal_config": {
            "db_host": "10.0.0.5",
            "db_pass": "P@ssw0rd1234!"
        }
    })


# اندپوینت کند و عمداً بی‌محتوا برای Lab 5 (blind SSRF زمان‌محور).
# هیچ فلگ یا داده‌ای برنمی‌گردونه — تنها سیگنال موفقیت، خودِ تأخیرِ پاسخه.
SLOW_CHECK_DELAY_SECONDS = 3

@app.route('/admin/slow-check', methods=['GET'])
def slow_check():
    time.sleep(SLOW_CHECK_DELAY_SECONDS)
    return jsonify({"status": "done"})


if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8080)