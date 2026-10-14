#!/usr/bin/env bash
# setup-drupal.sh — نصب خودکار Drupal برای CVE-2018-7600 (vulhub).
# یک بار اجرا کافیه. چون teardown: stop هست، بین ران‌ها باقی می‌مونه.

DRUPAL_DIR="${DRUPAL_DIR:-$HOME/vulhub-master/drupal/CVE-2018-7600}"
DRUSH_URL="https://github.com/drush-ops/drush/releases/download/8.4.12/drush.phar"

if [ ! -d "$DRUPAL_DIR" ]; then
    echo "❌ Drupal dir not found: $DRUPAL_DIR" >&2
    exit 1
fi

cd "$DRUPAL_DIR" || exit 1

echo "▶ 1/7  clean slate"
docker compose down -v 2>/dev/null
# docker ps -q | xargs -r docker kill 2>/dev/null

echo "▶ 2/7  up"
docker compose up -d || exit 1
sleep 5

echo "▶ 3/7  pre-check"
curl -sI http://localhost:8080/ | head -1

echo "▶ 4/7  drush site-install"
docker compose exec -T web bash -c "
    cd /var/www/html
    php -r \"copy('$DRUSH_URL', '/tmp/drush');\"
    chmod +x /tmp/drush
    /tmp/drush site-install standard \
        --db-url=sqlite://sites/default/files/.ht.sqlite \
        --account-name=admin --account-pass=admin \
        --site-name=test -y
"
echo "   (drush exit: $? — اگر warning داد مهم نیست)"

echo "▶ 5/7  fix permissions"
docker compose exec -T web bash -c "
    chown -R www-data:www-data /var/www/html/sites/default/
    chmod -R 755 /var/www/html/sites/default/
    chmod -R 775 /var/www/html/sites/default/files/
"

echo "▶ 6/7  clear twig cache"
docker compose exec -T web bash -c "
    rm -rf /var/www/html/sites/default/files/php/twig/* 2>/dev/null
"

echo "▶ 7/7  restart + verify"
docker compose restart >/dev/null
sleep 5
curl -sI http://localhost:8080/ | head -1

echo "✔ done."
