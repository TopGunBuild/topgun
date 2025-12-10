import { Link, Outlet, useLocation, useNavigate } from 'react-router-dom';
import { LayoutDashboard, Database, Server, LogOut } from 'lucide-react';

export function Layout() {
    const location = useLocation();
    const navigate = useNavigate();

    const handleLogout = () => {
        localStorage.removeItem('topgun_token');
        navigate('/login');
    };

    const isActive = (path: string) => location.pathname === path ? 'bg-gray-700' : '';

    return (
        <div className="flex h-screen bg-gray-100">
            {/* Sidebar */}
            <div className="w-64 bg-gray-800 text-white flex flex-col">
                <div className="p-6">
                    <h1 className="text-2xl font-bold">TopGun Admin</h1>
                </div>
                <nav className="flex-1 px-4 space-y-2">
                    <Link to="/" className={`flex items-center space-x-3 px-4 py-3 rounded hover:bg-gray-700 ${isActive('/')}`}>
                        <LayoutDashboard size={20} />
                        <span>Dashboard</span>
                    </Link>
                    <Link to="/maps" className={`flex items-center space-x-3 px-4 py-3 rounded hover:bg-gray-700 ${isActive('/maps')}`}>
                        <Database size={20} />
                        <span>Maps Explorer</span>
                    </Link>
                    <Link to="/cluster" className={`flex items-center space-x-3 px-4 py-3 rounded hover:bg-gray-700 ${isActive('/cluster')}`}>
                        <Server size={20} />
                        <span>Cluster</span>
                    </Link>
                </nav>
                <div className="p-4 border-t border-gray-700">
                    <button onClick={handleLogout} className="flex items-center space-x-3 px-4 py-3 w-full rounded hover:bg-gray-700 text-red-400 cursor-pointer">
                        <LogOut size={20} />
                        <span>Logout</span>
                    </button>
                </div>
            </div>

            {/* Main Content */}
            <div className="flex-1 overflow-auto">
                <Outlet />
            </div>
        </div>
    );
}
