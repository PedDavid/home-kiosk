package com.wesbos.homehq;

import android.app.admin.DeviceAdminReceiver;

/**
 * Device admin with one policy, force-lock, so the dashboard can put the screen fully to sleep
 * (DevicePolicyManager.lockNow, the same as the power button) while nobody is home or at night.
 * Without it the kiosk can only turn the backlight down to the panel's minimum. Enable once:
 *   adb shell dpm set-active-admin com.wesbos.homehq/.ScreenAdmin
 */
public class ScreenAdmin extends DeviceAdminReceiver {
}
