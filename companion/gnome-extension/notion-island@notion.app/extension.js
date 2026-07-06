/* Notion Dynamic Island — GNOME Shell extension (Component B).
 *
 * A top-bar indicator that mirrors the notion-watcher daemon's calendar agenda.
 * It builds a DBus proxy for com.notion.Calendar, shows the next event in the
 * panel, lists today's agenda in its menu, and refreshes instantly whenever the
 * daemon broadcasts EventsUpdated. It never touches the database itself — all
 * data arrives as JSON over DBus, so the extension stays tiny (~2 MB).
 */

import GObject from 'gi://GObject';
import St from 'gi://St';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Extension, gettext as _} from 'resource:///org/gnome/shell/extensions/extension.js';

const CALENDAR_IFACE = `
<node>
  <interface name="com.notion.Calendar">
    <method name="GetTodayEvents">
      <arg name="events" type="s" direction="out"/>
    </method>
    <method name="GetUpcoming">
      <arg name="count" type="i" direction="in"/>
      <arg name="events" type="s" direction="out"/>
    </method>
    <signal name="EventsUpdated">
      <arg name="json_data" type="s"/>
    </signal>
  </interface>
</node>`;

const CalendarProxy = Gio.DBusProxy.makeProxyWrapper(CALENDAR_IFACE);

/** Format a Unix-second start time (+ optional all-day flag) for display. */
function formatTime(event) {
    if (event.allDay)
        return _('All day');
    const dt = GLib.DateTime.new_from_unix_local(event.startTime);
    return dt ? dt.format('%H:%M') : '';
}

/** Human "Mon 3" style day label for grouping across today/tomorrow. */
function formatDay(event) {
    const dt = GLib.DateTime.new_from_unix_local(event.startTime);
    return dt ? dt.format('%a %-d') : '';
}

const NotionIndicator = GObject.registerClass(
class NotionIndicator extends PanelMenu.Button {
    _init(settings) {
        super._init(0.0, 'Notion Dynamic Island');
        this._settings = settings;
        this._events = [];
        this._signalId = 0;

        // Panel content: a small calendar glyph plus a live summary label.
        const box = new St.BoxLayout({style_class: 'notion-island-box'});
        this._icon = new St.Icon({
            icon_name: 'x-office-calendar-symbolic',
            style_class: 'system-status-icon notion-island-icon',
        });
        this._label = new St.Label({
            text: _('Calendar'),
            y_align: Clutter.ActorAlign.CENTER,
            style_class: 'notion-island-label',
        });
        box.add_child(this._icon);
        box.add_child(this._label);
        this.add_child(box);

        this._buildMenu([]);
        this._connectProxy();
    }

    _connectProxy() {
        this._proxy = new CalendarProxy(
            Gio.DBus.session,
            'com.notion.Calendar',
            '/com/notion/Calendar',
            (proxy, error) => {
                if (error) {
                    console.warn(`notion-island: could not reach daemon: ${error.message}`);
                    this._label.text = _('Notion');
                    return;
                }
                // Live updates + an initial pull so we render immediately.
                this._signalId = this._proxy.connectSignal(
                    'EventsUpdated', (_p, _sender, [json]) => this._render(json));
                this._proxy.GetTodayEventsRemote(([json], err) => {
                    if (!err && json)
                        this._render(json);
                });
            });
    }

    _render(json) {
        let events = [];
        try {
            events = JSON.parse(json) || [];
        } catch (e) {
            console.warn(`notion-island: bad event JSON: ${e}`);
            events = [];
        }
        events.sort((a, b) => a.startTime - b.startTime);
        this._events = events;
        this._updatePanel();
        this._buildMenu(events);
    }

    _updatePanel() {
        const now = GLib.get_real_time() / 1_000_000;
        const next = this._events.find(e => e.endTime > now);
        if (!next) {
            this._label.text = this._events.length ? _('Done for today') : _('No events');
            return;
        }
        const time = next.allDay ? '' : `${formatTime(next)} `;
        const title = next.title.length > 22 ? `${next.title.slice(0, 21)}…` : next.title;
        this._label.text = `${time}${title}`;
    }

    _buildMenu(events) {
        this.menu.removeAll();

        const header = new PopupMenu.PopupMenuItem(_('Today & Tomorrow'), {
            reactive: false,
            style_class: 'notion-island-header',
        });
        this.menu.addMenuItem(header);

        if (!events.length) {
            const empty = new PopupMenu.PopupMenuItem(_('No upcoming events'), {reactive: false});
            this.menu.addMenuItem(empty);
        } else {
            const max = this._settings.get_int('max-events');
            events.slice(0, max).forEach(event => {
                const item = new PopupMenu.PopupMenuItem('');
                const label = new St.Label({
                    text: `${formatDay(event)}  ${formatTime(event)}`,
                    style_class: 'notion-island-time',
                    y_align: Clutter.ActorAlign.CENTER,
                });
                const title = new St.Label({
                    text: event.title,
                    x_expand: true,
                    style_class: 'notion-island-event-title',
                    y_align: Clutter.ActorAlign.CENTER,
                });
                item.add_child(label);
                item.add_child(title);
                item.connect('activate', () => this._launchQuickView([]));
                this.menu.addMenuItem(item);
            });
        }

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        const open = new PopupMenu.PopupMenuItem(_('Open Quick View'));
        open.connect('activate', () => this._launchQuickView([]));
        this.menu.addMenuItem(open);

        const ask = new PopupMenu.PopupMenuItem(_('Ask AI ✨'));
        ask.connect('activate', () => this._launchQuickView(['--ask']));
        this.menu.addMenuItem(ask);
    }

    /** Launch the GTK quick-view app (Component C) with optional args. */
    _launchQuickView(extraArgs) {
        const command = this._settings.get_string('quickview-command') || 'notion-quickview';
        const argv = command.split(/\s+/).filter(s => s.length).concat(extraArgs);
        try {
            Gio.Subprocess.new(argv, Gio.SubprocessFlags.NONE);
        } catch (e) {
            Main.notifyError(_('Notion'), _('Could not launch the quick-view app.'));
            console.error(`notion-island: launch failed: ${e}`);
        }
    }

    destroy() {
        if (this._proxy && this._signalId) {
            this._proxy.disconnectSignal(this._signalId);
            this._signalId = 0;
        }
        this._proxy = null;
        super.destroy();
    }
});

export default class NotionIslandExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._indicator = new NotionIndicator(this._settings);
        Main.panel.addToStatusArea(this.uuid, this._indicator);
    }

    disable() {
        // Destroyed on lock screen too: the extension holds a DBus proxy and
        // must not leak it across sessions.
        this._indicator?.destroy();
        this._indicator = null;
        this._settings = null;
    }
}
