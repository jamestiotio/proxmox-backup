Ext.define('PBS.form.RemoteStoreSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsRemoteStoreSelector',

    queryMode: 'local',

    valueField: 'store',
    displayField: 'store',
    notFoundIsValid: true,

    matchFieldWidth: false,
    listConfig: {
	loadingText: gettext('Scanning...'),
	width: 350,
	columns: [
	    {
		header: gettext('Datastore'),
		sortable: true,
		dataIndex: 'store',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },

    doRawQuery: function() {
	// do nothing.
    },

    setRemote: function(remote) {
	let me = this;

	if (me.remote === remote) {
	    return;
	}

	me.remote = remote;

	me.store.removeAll();

	if (me.remote) {
	    me.setDisabled(false);
	    if (!me.firstLoad) {
		me.clearValue();
	    }

	    me.store.proxy.url = `/api2/json/config/remote/${encodeURIComponent(me.remote)}/scan`;
	    me.store.load();

	    me.firstLoad = false;
	} else {
	    me.setDisabled(true);
	    me.clearValue();
	}
    },

    initComponent: function() {
	let me = this;

	me.firstLoad = true;

	let store = Ext.create('Ext.data.Store', {
	    fields: ['store', 'comment'],
	    proxy: {
		type: 'proxmox',
		url: '/api2/json/config/remote/' + encodeURIComponent(me.remote) + '/scan',
	    },
	});

	store.sort('store', 'ASC');

	Ext.apply(me, {
	    store: store,
	});

	me.callParent();
    },
});

Ext.define('PBS.form.RemoteNamespaceSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsRemoteNamespaceSelector',

    queryMode: 'local',

    valueField: 'ns',
    displayField: 'ns',
    emptyText: gettext('Root'),
    notFoundIsValid: true,

    matchFieldWidth: false,
    listConfig: {
	loadingText: gettext('Scanning...'),
	width: 350,
	columns: [
	    {
		header: gettext('Namespace'),
		sortable: true,
		dataIndex: 'ns',
		renderer: PBS.Utils.render_optional_namespace,
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },

    doRawQuery: function() {
	// do nothing.
    },

    setRemote: function(remote) {
	let me = this;
	let previousRemote = me.remote;
	if (previousRemote === remote) {
	    return;
	}
	me.remote = remote;

	me.store.removeAll();

	if (previousRemote) {
	    me.setDisabled(true);
	    me.clearValue();
	}
    },

    setRemoteStore: function(remoteStore) {
	let me = this;
	let previousStore = me.remoteStore;
	if (previousStore === remoteStore) {
	    return;
	}
	me.remoteStore = remoteStore;

	me.store.removeAll();

	if (me.remote && me.remoteStore) {
	    me.setDisabled(false);
	    if (!me.firstLoad) {
		me.clearValue();
	    }
	    let encodedRemote = encodeURIComponent(me.remote);
	    let encodedStore = encodeURIComponent(me.remoteStore);

	    me.store.proxy.url = `/api2/json/config/remote/${encodedRemote}/scan/${encodedStore}/namespaces`;
	    me.store.load();

	    me.firstLoad = false;
	} else if (previousStore) {
	    me.setDisabled(true);
	    me.clearValue();
	}
    },

    initComponent: function() {
	let me = this;

	me.firstLoad = true;

	let store = Ext.create('Ext.data.Store', {
	    fields: ['ns', 'comment'],
	    proxy: {
		type: 'proxmox',
		url: `/api2/json/config/remote/${encodeURIComponent(me.remote)}/scan`,
	    },
	});
	store.sort('ns', 'ASC');

	Ext.apply(me, {
	    store: store,
	});

	me.callParent();
    },
});
