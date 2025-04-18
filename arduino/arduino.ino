#include <LiquidCrystal.h>

LiquidCrystal lcd(2, 3, 4, 5, 6, 7);

void setup() {
  Serial.begin(9600);

  lcd.begin(16, 2);
  lcd.setCursor(0, 0);
  lcd.print("Cat Pics Radar");

  lcd.setCursor(0, 1);
  lcd.print("Searching...");
}

void loop() {
  if (Serial.available() > 0) {
    String data = Serial.readStringUntil('\n');
    // well formed "packet" must have a `,` so we can split into filename
    // and message
    int idx = data.indexOf(',');
    if (idx < 0) {
      return;
    }
    String filename = data.substring(0, idx);
    String msg = data.substring(idx + 1, data.length());

    lcd.setCursor(0, 0);
    lcd.clear();
    lcd.print(filename);

    lcd.setCursor(0, 1);
    lcd.print(msg);
  }
}
