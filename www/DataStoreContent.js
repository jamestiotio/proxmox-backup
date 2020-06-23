Ext.define('pbs-data-store-snapshots', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-type',
	'backup-id',
	{
	    name: 'backup-time',
	    type: 'date',
	    dateFormat: 'timestamp'
	},
	'files',
	'owner',
	{ name: 'size', type: 'int' },
	{
	    name: 'encrypted',
	    type: 'boolean',
	    calculate: function(data) {
		let encrypted = 0;
		let files = 0;
		data.files.forEach(file => {
		    if (file.filename === 'index.json.blob') return; // is never encrypted
		    if (file.encrypted) {
			encrypted++;
		    }
		    files++;
		});

		if (encrypted === 0) {
		    return 0;
		} else if (encrypted < files) {
		    return 1;
		} else {
		    return 2;
		}
	    }
	}
    ]
});

Ext.define('PBS.DataStoreContent', {
    extend: 'Ext.tree.Panel',
    alias: 'widget.pbsDataStoreContent',

    rootVisible: false,

    title: gettext('Content'),

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    if (!view.datastore) {
		throw "no datastore specified";
	    }

	    this.store = Ext.create('Ext.data.Store', {
		model: 'pbs-data-store-snapshots',
		sorters: 'backup-group',
		groupField: 'backup-group',
	    });
	    this.store.on('load', this.onLoad, this);

	    Proxmox.Utils.monStoreErrors(view, view.store, true);
	    this.reload(); // initial load
	},

	reload: function() {
	    let view = this.getView();

	    if (!view.store || !this.store) {
		console.warn('cannot reload, no store(s)');
		return;
	    }

	    let url = `/api2/json/admin/datastore/${view.datastore}/snapshots`;
	    this.store.setProxy({
		type: 'proxmox',
		url:  url
	    });

	    this.store.load();
	},

	getRecordGroups: function(records) {
	    let groups = {};

	    for (const item of records) {
		var btype = item.data["backup-type"];
		let group = btype + "/" + item.data["backup-id"];

		if (groups[group] !== undefined) {
		    continue;
		}

		var cls = '';
		if (btype === 'vm') {
		    cls = 'fa-desktop';
		} else if (btype === 'ct') {
		    cls = 'fa-cube';
		} else if (btype === 'host') {
		    cls = 'fa-building';
		} else {
		    console.warn(`got unknown backup-type '${btype}'`);
		    continue; // FIXME: auto render? what do?
		}

		groups[group] = {
		    text: group,
		    leaf: false,
		    iconCls: "fa " + cls,
		    expanded: false,
		    backup_type: item.data["backup-type"],
		    backup_id: item.data["backup-id"],
		    children: []
		};
	    }

	    return groups;
	},

	onLoad: function(store, records, success) {
	    let view = this.getView();

	    if (!success) {
		return;
	    }

	    let groups = this.getRecordGroups(records);

	    for (const item of records) {
		let group = item.data["backup-type"] + "/" + item.data["backup-id"];
		let children = groups[group].children;

		let data = item.data;

		data.text = group + '/' + PBS.Utils.render_datetime_utc(data["backup-time"]);
		data.leaf = true;
		data.cls = 'no-leaf-icons';

		children.push(data);
	    }

	    let children = [];
	    for (const [_key, group] of Object.entries(groups)) {
		let last_backup = 0;
		let encrypted = 0;
		for (const item of group.children) {
		    if (item.encrypted > 0) {
			encrypted++;
		    }
		    if (item["backup-time"] > last_backup) {
			last_backup = item["backup-time"];
			group["backup-time"] = last_backup;
			group.files = item.files;
			group.size = item.size;
			group.owner = item.owner;
		    }

		}
		if (encrypted === 0) {
		    group.encrypted = 0;
		} else if (encrypted < group.children.length) {
		    group.encrypted = 1;
		} else {
		    group.encrypted = 2;
		}
		group.count = group.children.length;
		children.push(group);
	    }

	    view.setRootNode({
		expanded: true,
		children: children
	    });
	},

	onPrune: function() {
	    var view = this.getView();

	    let rec = view.selModel.getSelection()[0];
	    if (!(rec && rec.data)) return;
	    let data = rec.data;
	    if (data.leaf) return;

	    if (!view.datastore) return;

	    let win = Ext.create('PBS.DataStorePrune', {
		datastore: view.datastore,
		backup_type: data.backup_type,
		backup_id: data.backup_id,
	    });
	    win.on('destroy', this.reload, this);
	    win.show();
	}
    },

    columns: [
	{
	    xtype: 'treecolumn',
	    header: gettext("Backup Group"),
	    dataIndex: 'text',
	    flex: 1
	},
	{
	    xtype: 'datecolumn',
	    header: gettext('Backup Time'),
	    sortable: true,
	    dataIndex: 'backup-time',
	    format: 'Y-m-d H:i:s',
	    width: 150
	},
	{
	    header: gettext("Size"),
	    sortable: true,
	    dataIndex: 'size',
	    renderer: Proxmox.Utils.format_size,
	},
	{
	    xtype: 'numbercolumn',
	    format: '0',
	    header: gettext("Count"),
	    sortable: true,
	    dataIndex: 'count',
	},
	{
	    header: gettext("Owner"),
	    sortable: true,
	    dataIndex: 'owner',
	},
	{
	    header: gettext('Encrypted'),
	    dataIndex: 'encrypted',
	    renderer: function(value) {
		switch (value) {
		    case 0: return Proxmox.Utils.noText;
		    case 1: return gettext('Mixed');
		    case 2: return Proxmox.Utils.yesText;
		    default: Proxmox.Utils.unknownText;
		}
	    }
	},
	{
	    header: gettext("Files"),
	    sortable: false,
	    dataIndex: 'files',
	    renderer: function(files) {
		return files.map((file) => {
		    let icon = '';
		    let size = '';
		    if (file.encrypted) {
			icon = '<i class="fa fa-lock"></i> ';
		    }
		    if (file.size)  {
			size = ` (${Proxmox.Utils.format_size(file.size)})`;
		    }
		    return `${icon}${file.filename}${size}`;
		}).join(', ');
	    },
	    flex: 2
	},
    ],

    tbar: [
	{
	    text: gettext('Reload'),
	    iconCls: 'fa fa-refresh',
	    handler: 'reload',
	},
	{
	    xtype: 'proxmoxButton',
	    text: gettext('Prune'),
	    disabled: true,
	    parentXType: 'pbsDataStoreContent',
	    enableFn: function(record) { return !record.data.leaf; },
	    handler: 'onPrune',
	}
    ],
});
