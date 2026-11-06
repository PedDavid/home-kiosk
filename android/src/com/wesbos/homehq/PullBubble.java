package com.wesbos.homehq;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.RectF;
import android.view.View;

/**
 * The round "pull down for controls" indicator. While the pull is short of the trigger distance a
 * ring fills around a menu icon; once it is full the bubble turns light, and letting go opens the
 * controls. Drawn by hand so it needs no resources or icon fonts.
 */
class PullBubble extends View {
  private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);
  private final Paint track = new Paint(Paint.ANTI_ALIAS_FLAG);
  private final Paint ring = new Paint(Paint.ANTI_ALIAS_FLAG);
  private final Paint icon = new Paint(Paint.ANTI_ALIAS_FLAG);
  private final RectF arc = new RectF();
  private float progress;
  private boolean armed;

  PullBubble(Context context) {
    super(context);
    float density = context.getResources().getDisplayMetrics().density;
    track.setStyle(Paint.Style.STROKE);
    track.setStrokeWidth(3 * density);
    track.setColor(Color.argb(70, 255, 255, 255));
    ring.setStyle(Paint.Style.STROKE);
    ring.setStrokeWidth(3 * density);
    ring.setStrokeCap(Paint.Cap.ROUND);
    ring.setColor(Color.WHITE);
    icon.setStrokeWidth(2.5f * density);
    icon.setStrokeCap(Paint.Cap.ROUND);
  }

  /** @param progress 0..1 of the way to the trigger distance; 1 means letting go opens the controls. */
  void setProgress(float progress) {
    progress = Math.max(0, Math.min(1, progress));
    boolean nowArmed = progress >= 1;
    if (progress == this.progress && nowArmed == armed) return;
    this.progress = progress;
    if (nowArmed != armed) {
      armed = nowArmed;
      animate().scaleX(armed ? 1.15f : 1f).scaleY(armed ? 1.15f : 1f).setDuration(120).start();
    }
    invalidate();
  }

  boolean isArmed() {
    return armed;
  }

  @Override
  protected void onDraw(Canvas canvas) {
    float w = getWidth(), h = getHeight();
    float cx = w / 2, cy = h / 2;
    float inset = ring.getStrokeWidth();
    float r = Math.min(w, h) / 2 - inset;
    fill.setColor(armed ? Color.argb(240, 245, 245, 245) : Color.argb(220, 30, 30, 30));
    canvas.drawCircle(cx, cy, r, fill);
    if (!armed) {
      arc.set(cx - r, cy - r, cx + r, cy + r);
      canvas.drawOval(arc, track);
      canvas.drawArc(arc, -90, 360 * progress, false, ring);
    }
    // Three-line menu icon.
    icon.setColor(armed ? Color.rgb(30, 30, 30) : Color.WHITE);
    float half = r * 0.42f, gap = r * 0.3f;
    for (int i = -1; i <= 1; i++) {
      canvas.drawLine(cx - half, cy + i * gap, cx + half, cy + i * gap, icon);
    }
  }
}
